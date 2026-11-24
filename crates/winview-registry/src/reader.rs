// winview-registry/src/reader.rs

use crate::version::detect_windows_version;
use std::collections::BTreeMap;
use std::ops::Deref;
use windows_registry::{Key, Type as RegType, CURRENT_USER, LOCAL_MACHINE};
use winview_core::model::{
    DecodingStatus, FolderTypeSnapshot, FolderTypesSnapshot, InspectionError, InspectionSnapshot,
    InspectionStatus, KeySnapshot, ObservationSource, RawRegistryValue, ReadError, RegistryValue,
    ValueReadResult, ViewSnapshot,
};
use winview_core::registry::keys::{RootKey, ValueTypeName};
use winview_core::registry::traits::{RegistryError, RegistryKey, RegistryReader, RegistryResult};

fn map_windows_error<E: std::fmt::Display>(e: E, root: RootKey, path: &str) -> RegistryError {
    let msg = e.to_string();
    let lower = msg.to_lowercase();
    if lower.contains("not found") || lower.contains("cannot find") {
        RegistryError::KeyNotFound {
            root,
            path: path.to_string(),
        }
    } else if lower.contains("access denied") || lower.contains("permission") {
        RegistryError::AccessDenied {
            root,
            path: path.to_string(),
        }
    } else {
        RegistryError::WindowsApi(msg)
    }
}

pub const MAX_DEPTH: usize = 32;

const MAX_KEYS_PER_DOMAIN: usize = 50_000;

const MAX_VALUES_PER_KEY: usize = 1_000;

const MAX_VALUE_BYTES: usize = 1024 * 1024;

const MAX_SNAPSHOT_BYTES: usize = 50 * 1024 * 1024;

struct SnapshotBudget {
    captured_bytes: usize,
    max_bytes: usize,
}

impl SnapshotBudget {
    fn new(max_bytes: usize) -> Self {
        Self {
            captured_bytes: 0,
            max_bytes,
        }
    }

    fn try_capture(&mut self, bytes: usize) -> bool {
        let Some(next) = self.captured_bytes.checked_add(bytes) else {
            return false;
        };

        if next > self.max_bytes {
            return false;
        }

        self.captured_bytes = next;
        true
    }
}

struct TraversalBudget {
    visited_keys: usize,
    max_keys: usize,
    limit_exceeded: bool,
    max_values_per_key: usize,
    max_value_bytes: usize,
}

impl TraversalBudget {
    fn new(max_keys: usize) -> Self {
        Self {
            visited_keys: 0,
            max_keys,
            limit_exceeded: false,
            max_values_per_key: MAX_VALUES_PER_KEY,
            max_value_bytes: MAX_VALUE_BYTES,
        }
    }

    fn record_visit(&mut self) {
        self.visited_keys += 1;
    }

    fn is_exhausted(&self) -> bool {
        self.visited_keys >= self.max_keys
    }
}

pub struct WindowsRegistryReader;

impl RegistryReader for WindowsRegistryReader {
    fn open_key(&self, root: RootKey, path: &str) -> RegistryResult<Box<dyn RegistryKey>> {
        let key = match root {
            RootKey::Hkcu => CURRENT_USER
                .open(path)
                .map_err(|e| map_windows_error(e, root, path))?,
            RootKey::Hklm => LOCAL_MACHINE
                .open(path)
                .map_err(|e| map_windows_error(e, root, path))?,
        };
        Ok(Box::new(WindowsRegistryKey(key)))
    }

    fn enum_keys(&self, root: RootKey, path: &str) -> RegistryResult<Vec<String>> {
        let key = self.open_key(root, path)?;
        key.enum_subkeys()
    }

    fn get_value(
        &self,
        root: RootKey,
        path: &str,
        name: &str,
    ) -> RegistryResult<(ValueTypeName, Vec<u8>, RegistryValue, DecodingStatus)> {
        let key = self.open_key(root, path)?;
        key.get_value(name)
    }

    fn get_value_type(
        &self,
        root: RootKey,
        path: &str,
        name: &str,
    ) -> RegistryResult<ValueTypeName> {
        let key = self.open_key(root, path)?;
        key.get_value_type(name)
    }

    fn inspect_all(&self) -> RegistryResult<InspectionSnapshot> {
        let mut snapshot = InspectionSnapshot::default();

        // Shared snapshot-wide capture budget across all inspection domains.
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        // Timestamp — populated by adapter (core doesn't need chrono)
        snapshot.metadata.captured_at = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();

        // Windows version detection (in this crate, not core)
        let version_info = detect_windows_version();
        snapshot.metadata.windows_build = version_info.build;
        snapshot.metadata.windows_ubr = version_info.ubr;
        snapshot.metadata.windows_product_name = version_info.product_name;
        snapshot.metadata.detection_method = version_info.detection_method;

        // Inspect each domain, collecting errors
        inspect_folder_types(self, &mut snapshot, &mut snapshot_budget);
        inspect_bags(self, &mut snapshot);
        inspect_streams(self, &mut snapshot);
        inspect_advanced(self, &mut snapshot, MAX_VALUE_BYTES, &mut snapshot_budget);
        inspect_clsid(self, &mut snapshot);
        inspect_policies(self, &mut snapshot);

        // Determine inspection status based on collected errors
        let total_errors = snapshot.errors.len()
            + snapshot.hkcu_folder_types.read_errors.len()
            + snapshot.hklm_folder_types.read_errors.len()
            + snapshot.advanced.read_errors.len();
        let has_data = !snapshot.hkcu_folder_types.types.is_empty()
            || !snapshot.hklm_folder_types.types.is_empty()
            || !snapshot.bags.is_empty()
            || !snapshot.streams.is_empty()
            || !snapshot.advanced.values.is_empty()
            || !snapshot.clsid.is_empty()
            || !snapshot.policies.is_empty();

        if total_errors == 0 {
            snapshot.status = InspectionStatus::Complete;
        } else if has_data {
            snapshot.status = InspectionStatus::Partial {
                reason: format!("{} read errors encountered", total_errors),
            };
        } else {
            snapshot.status = InspectionStatus::Failed {
                error: format!("Inspection failed with {} errors", total_errors),
            };
        }

        finalize_inspection_status(&mut snapshot);
        Ok(snapshot)
    }
}

struct WindowsRegistryKey(Key);

impl RegistryKey for WindowsRegistryKey {
    fn enum_values(&self) -> RegistryResult<Vec<(String, ValueTypeName)>> {
        let iter = self
            .0
            .values()
            .map_err(|e| RegistryError::WindowsApi(e.to_string()))?;

        let mut results = Vec::new();
        for (name, value) in iter {
            let type_name = convert_type(value.ty());
            results.push((name, type_name));
        }
        Ok(results)
    }

    fn enum_subkeys(&self) -> RegistryResult<Vec<String>> {
        let iter = self
            .0
            .keys()
            .map_err(|e| RegistryError::WindowsApi(e.to_string()))?;

        let mut results = Vec::new();
        for name in iter {
            results.push(name);
        }
        Ok(results)
    }

    fn get_value(
        &self,
        name: &str,
    ) -> RegistryResult<(ValueTypeName, Vec<u8>, RegistryValue, DecodingStatus)> {
        let value = self
            .0
            .get_value(name)
            .map_err(|e| RegistryError::WindowsApi(e.to_string()))?;

        let (type_name, reg_value, raw_bytes, status) =
            convert_registry_value(value.ty(), value.deref());
        Ok((type_name, raw_bytes, reg_value, status))
    }

    fn get_value_type(&self, name: &str) -> RegistryResult<ValueTypeName> {
        let vtype = self
            .0
            .get_type(name)
            .map_err(|e| RegistryError::WindowsApi(e.to_string()))?;

        Ok(convert_type(vtype))
    }

    fn open_subkey(&self, name: &str) -> RegistryResult<Box<dyn RegistryKey>> {
        let subkey = self
            .0
            .open(name)
            .map_err(|e| RegistryError::WindowsApi(e.to_string()))?;
        Ok(Box::new(WindowsRegistryKey(subkey)))
    }
}

// Convert windows_registry::Type to our ValueTypeName
fn convert_type(vtype: RegType) -> ValueTypeName {
    match vtype {
        RegType::String => ValueTypeName::RegSz,
        RegType::ExpandString => ValueTypeName::RegExpandSz,
        RegType::U32 => ValueTypeName::RegDword,
        RegType::U64 => ValueTypeName::RegQword,
        RegType::Bytes => ValueTypeName::RegBinary,
        RegType::MultiString => ValueTypeName::RegMultiSz,
        RegType::Other(id) => ValueTypeName::Other(id), // Preserve numeric ID
    }
}

// Convert windows_registry Value to our platform-neutral types
// Returns: (ValueTypeName, RegistryValue, raw_bytes, DecodingStatus)
fn convert_registry_value(
    vtype: RegType,
    data: &[u8],
) -> (ValueTypeName, RegistryValue, Vec<u8>, DecodingStatus) {
    let type_name = convert_type(vtype);
    let _raw_type_id = u32::from(vtype);
    let raw_bytes = data.to_vec(); // ALWAYS preserve raw bytes

    match vtype {
        RegType::String => match decode_utf16_le(data) {
            Ok(s) => (
                type_name,
                RegistryValue::String(s),
                raw_bytes,
                DecodingStatus::Decoded,
            ),
            Err(e) => (
                type_name,
                RegistryValue::Binary(data.to_vec()),
                raw_bytes,
                DecodingStatus::Malformed { reason: e },
            ),
        },
        RegType::ExpandString => match decode_utf16_le(data) {
            Ok(s) => (
                type_name,
                RegistryValue::ExpandString(s),
                raw_bytes,
                DecodingStatus::Decoded,
            ),
            Err(e) => (
                type_name,
                RegistryValue::Binary(data.to_vec()),
                raw_bytes,
                DecodingStatus::Malformed { reason: e },
            ),
        },
        RegType::U32 => {
            if data.len() != 4 {
                return (
                    type_name,
                    RegistryValue::Binary(data.to_vec()),
                    raw_bytes,
                    DecodingStatus::Malformed {
                        reason: "DWORD length != 4".into(),
                    },
                );
            }
            let val = u32::from_le_bytes(data.try_into().unwrap());
            (
                type_name,
                RegistryValue::Dword(val),
                raw_bytes,
                DecodingStatus::Decoded,
            )
        }
        RegType::U64 => {
            if data.len() != 8 {
                return (
                    type_name,
                    RegistryValue::Binary(data.to_vec()),
                    raw_bytes,
                    DecodingStatus::Malformed {
                        reason: "QWORD length != 8".into(),
                    },
                );
            }
            let val = u64::from_le_bytes(data.try_into().unwrap());
            (
                type_name,
                RegistryValue::Qword(val),
                raw_bytes,
                DecodingStatus::Decoded,
            )
        }
        RegType::Bytes => (
            type_name,
            RegistryValue::Binary(data.to_vec()),
            raw_bytes,
            DecodingStatus::Decoded,
        ),
        RegType::MultiString => match decode_reg_multi_sz(data) {
            Ok(parts) => (
                type_name,
                RegistryValue::MultiString(parts),
                raw_bytes,
                DecodingStatus::Decoded,
            ),
            Err(e) => (
                type_name,
                RegistryValue::Binary(data.to_vec()),
                raw_bytes,
                DecodingStatus::Malformed { reason: e },
            ),
        },
        RegType::Other(id) => {
            // Preserve unknown types as raw bytes with Windows type ID
            // Includes REG_NONE as Other(0)
            (
                ValueTypeName::Other(id),
                RegistryValue::Binary(data.to_vec()),
                raw_bytes,
                DecodingStatus::UnknownType,
            )
        }
    }
}

// UTF-16LE decoding — returns error on malformed data, preserves raw bytes in caller
fn decode_utf16_le(data: &[u8]) -> Result<String, String> {
    if !data.len().is_multiple_of(2) {
        return Err("Odd byte length for UTF-16".into());
    }
    let u16s: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    // Find null terminator if present
    let end = u16s.iter().position(|&c| c == 0).unwrap_or(u16s.len());
    let valid = &u16s[..end];
    String::from_utf16(valid).map_err(|_| "Invalid UTF-16 sequence".to_string())
}

/// Dedicated REG_MULTI_SZ decoder — preserves all elements including empty strings
/// Windows semantics: double-null terminates; single-null separates; empty multi-string = zero-length (no strings)
fn decode_reg_multi_sz(data: &[u8]) -> Result<Vec<String>, String> {
    if !data.len().is_multiple_of(2) {
        return Err("Odd byte length for REG_MULTI_SZ".into());
    }
    let u16s: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();

    let mut result = Vec::new();
    let mut current = Vec::new();
    let mut i = 0;

    // Empty buffer = empty multi-string
    if u16s.is_empty() {
        return Ok(Vec::new());
    }

    while i < u16s.len() {
        let c = u16s[i];
        if c == 0 {
            // Null terminator - check for double-null (end of multi-string)
            if i + 1 < u16s.len() && u16s[i + 1] == 0 {
                // Double null - end of REG_MULTI_SZ
                // Push the current string (may be empty)
                result.push(
                    String::from_utf16(&current)
                        .map_err(|_| "Invalid UTF-16 in multi-string element".to_string())?,
                );
                break;
            } else {
                // Single null - separator between strings
                result.push(
                    String::from_utf16(&current)
                        .map_err(|_| "Invalid UTF-16 in multi-string element".to_string())?,
                );
                current.clear();
                i += 1;
                continue;
            }
        }
        current.push(c);
        i += 1;
    }

    // If we exited without double-null, the data is malformed
    if i >= u16s.len() {
        return Err("Missing double-null terminator".into());
    }

    // Trailing data after double-null is ignored per Windows behavior
    Ok(result)
}

// Inspection modules - collect errors instead of failing
fn inspect_folder_types(
    reader: &WindowsRegistryReader,
    snapshot: &mut InspectionSnapshot,
    snapshot_budget: &mut SnapshotBudget,
) {
    use winview_core::registry::keys::paths::{HKCU_FOLDER_TYPES, HKLM_FOLDER_TYPES};
    inspect_folder_types_hive(
        reader,
        RootKey::Hkcu,
        HKCU_FOLDER_TYPES,
        ObservationSource::HkcuFolderTypes,
        &mut snapshot.hkcu_folder_types,
        MAX_VALUE_BYTES,
        snapshot_budget,
    );

    // HKLM FolderTypes
    inspect_folder_types_hive(
        reader,
        RootKey::Hklm,
        HKLM_FOLDER_TYPES,
        ObservationSource::HklmFolderTypes,
        &mut snapshot.hklm_folder_types,
        MAX_VALUE_BYTES,
        snapshot_budget,
    );
}

fn inspect_folder_types_hive(
    reader: &dyn RegistryReader,
    root: RootKey,
    path: &str,
    source: ObservationSource,
    folder_types: &mut FolderTypesSnapshot,
    max_value_bytes: usize,
    snapshot_budget: &mut SnapshotBudget,
) {
    use winview_core::registry::keys::paths::{
        CANONICAL_NAME, COLUMN_LIST, COL_INFO, F_FLAGS, GROUP_ASCENDING, GROUP_BY, ICON_SIZE,
        LOGICAL_VIEW_MODE, MODE, SORT_BY_LIST,
    };

    // Open the FolderTypes key
    let key_result = reader.open_key(root, path);
    let key = match key_result {
        Ok(k) => k,
        Err(e) => {
            folder_types.read_errors.push(InspectionError {
                key_path: path.to_string(),
                root,
                operation: "open_key".to_string(),
                error: e.to_string(),
            });
            return;
        }
    };

    // Enumerate subkeys (folder type GUIDs)
    let guids = match key.enum_subkeys() {
        Ok(g) => g,
        Err(e) => {
            folder_types.read_errors.push(InspectionError {
                key_path: path.to_string(),
                root,
                operation: "enum_subkeys".to_string(),
                error: e.to_string(),
            });
            return;
        }
    };

    for guid in guids {
        let guid_path = format!(r"{}\{}", path, guid);
        let guid_key_result = reader.open_key(root, &guid_path);
        let guid_key = match guid_key_result {
            Ok(k) => k,
            Err(e) => {
                folder_types.read_errors.push(InspectionError {
                    key_path: guid_path,
                    root,
                    operation: "open_key".to_string(),
                    error: e.to_string(),
                });
                continue;
            }
        };

        // Read CanonicalName
        let canonical_name = match guid_key.get_value(CANONICAL_NAME) {
            Ok((_, raw_bytes, decoded, status)) => {
                if let Some(error) = check_value_size(
                    root,
                    &guid_path,
                    CANONICAL_NAME,
                    raw_bytes.len(),
                    max_value_bytes,
                ) {
                    folder_types.read_errors.push(error);
                    None
                } else if status == DecodingStatus::Decoded {
                    match decoded {
                        RegistryValue::String(s) => {
                            if let Some(error) = check_snapshot_budget(
                                snapshot_budget,
                                root,
                                &guid_path,
                                CANONICAL_NAME,
                                raw_bytes.len(),
                            ) {
                                folder_types.read_errors.push(error);
                                None
                            } else {
                                Some(s)
                            }
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            }
            Err(_) => None,
        };

        // Open TopViews\{00000000-0000-0000-0000-000000000000}
        let top_views_path = format!(
            r"{}\TopViews\{{00000000-0000-0000-0000-000000000000}}",
            guid_path
        );
        let top_views_key_result = reader.open_key(root, &top_views_path);
        let top_views_key = match top_views_key_result {
            Ok(k) => k,
            Err(e) => {
                folder_types.read_errors.push(InspectionError {
                    key_path: top_views_path,
                    root,
                    operation: "open_key".to_string(),
                    error: e.to_string(),
                });
                continue;
            }
        };

        // Read all view values
        let mut raw_values = BTreeMap::new();
        let view_value_names = [
            (LOGICAL_VIEW_MODE, "LogicalViewMode"),
            (MODE, "Mode"),
            (ICON_SIZE, "IconSize"),
            (GROUP_BY, "GroupBy"),
            (GROUP_ASCENDING, "GroupAscending"),
            (SORT_BY_LIST, "SortByList"),
            (COLUMN_LIST, "ColumnList"),
            (COL_INFO, "ColInfo"),
            (F_FLAGS, "FFlags"),
        ];

        for (const_name, field_name) in view_value_names {
            let value_result = top_views_key.get_value(const_name);
            let read_result = match value_result {
                Ok((tn, raw_bytes, decoded, status)) => {
                    if let Some(error) = check_value_size(
                        root,
                        &top_views_path,
                        const_name,
                        raw_bytes.len(),
                        max_value_bytes,
                    ) {
                        folder_types.read_errors.push(error);
                        continue;
                    }

                    if let Some(error) = check_snapshot_budget(
                        snapshot_budget,
                        root,
                        &top_views_path,
                        const_name,
                        raw_bytes.len(),
                    ) {
                        folder_types.read_errors.push(error);
                        continue;
                    }

                    let decoded_opt = match status {
                        DecodingStatus::Decoded => Some(decoded),
                        _ => None,
                    };
                    ValueReadResult::Success(RawRegistryValue {
                        key_path: top_views_path.clone(),
                        value_name: field_name.to_string(),
                        root,
                        raw_type_id: tn.to_raw_id(),
                        raw_bytes,
                        decoded: decoded_opt,
                        decoding_status: status,
                        source,
                    })
                }
                Err(e) => ValueReadResult::Failure(ReadError::Other(e.to_string())),
            };
            raw_values.insert(field_name.to_string(), read_result);
        }

        let view_snapshot = ViewSnapshot {
            logical_view_mode: raw_values.get("LogicalViewMode").and_then(|v| match v {
                ValueReadResult::Success(r) => r.decoded.as_ref().and_then(|d| match d {
                    RegistryValue::Dword(v) => Some(*v),
                    _ => None,
                }),
                _ => None,
            }),
            mode: raw_values.get("Mode").and_then(|v| match v {
                ValueReadResult::Success(r) => r.decoded.as_ref().and_then(|d| match d {
                    RegistryValue::Dword(v) => Some(*v),
                    _ => None,
                }),
                _ => None,
            }),
            icon_size: raw_values.get("IconSize").and_then(|v| match v {
                ValueReadResult::Success(r) => r.decoded.as_ref().and_then(|d| match d {
                    RegistryValue::Dword(v) => Some(*v),
                    _ => None,
                }),
                _ => None,
            }),
            group_by: raw_values.get("GroupBy").and_then(|v| match v {
                ValueReadResult::Success(r) => r.decoded.as_ref().and_then(|d| match d {
                    RegistryValue::String(s) => Some(s.clone()),
                    _ => None,
                }),
                _ => None,
            }),
            group_ascending: raw_values.get("GroupAscending").and_then(|v| match v {
                ValueReadResult::Success(r) => r.decoded.as_ref().and_then(|d| match d {
                    RegistryValue::Dword(v) => Some(*v),
                    _ => None,
                }),
                _ => None,
            }),
            sort_by_list: raw_values.get("SortByList").and_then(|v| match v {
                ValueReadResult::Success(r) => r.decoded.as_ref().and_then(|d| match d {
                    RegistryValue::String(s) => Some(s.clone()),
                    _ => None,
                }),
                _ => None,
            }),
            column_list: raw_values.get("ColumnList").and_then(|v| match v {
                ValueReadResult::Success(r) => r.decoded.as_ref().and_then(|d| match d {
                    RegistryValue::String(s) => Some(s.clone()),
                    _ => None,
                }),
                _ => None,
            }),
            col_info: raw_values.get("ColInfo").and_then(|v| match v {
                ValueReadResult::Success(r) => r.decoded.as_ref().and_then(|d| match d {
                    RegistryValue::Binary(b) => Some(b.clone()),
                    _ => None,
                }),
                _ => None,
            }),
            f_flags: raw_values.get("FFlags").and_then(|v| match v {
                ValueReadResult::Success(r) => r.decoded.as_ref().and_then(|d| match d {
                    RegistryValue::Dword(v) => Some(*v),
                    _ => None,
                }),
                _ => None,
            }),
            raw_values,
        };

        let folder_type_snapshot = FolderTypeSnapshot {
            guid: guid.clone(),
            canonical_name,
            top_views: {
                let mut map = BTreeMap::new();
                map.insert(
                    "{00000000-0000-0000-0000-000000000000}".to_string(),
                    view_snapshot,
                );
                map
            },
            read_errors: Vec::new(),
        };

        folder_types.types.insert(guid, folder_type_snapshot);
    }
}

fn inspect_bags(reader: &WindowsRegistryReader, snapshot: &mut InspectionSnapshot) {
    use winview_core::registry::keys::paths::{
        ALL_FOLDERS_COMDLG, ALL_FOLDERS_COMDLG_LEGACY, ALL_FOLDERS_SHELL, BAGS, BAG_MRU,
    };

    let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);

    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        BAG_MRU,
        ObservationSource::HkcuBags,
        &mut snapshot.bags,
        "",
        0,
        &mut budget,
    );
    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        BAGS,
        ObservationSource::HkcuBags,
        &mut snapshot.bags,
        "",
        0,
        &mut budget,
    );
    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        ALL_FOLDERS_SHELL,
        ObservationSource::HkcuBags,
        &mut snapshot.bags,
        "",
        0,
        &mut budget,
    );
    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        ALL_FOLDERS_COMDLG,
        ObservationSource::HkcuBags,
        &mut snapshot.bags,
        "",
        0,
        &mut budget,
    );
    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        ALL_FOLDERS_COMDLG_LEGACY,
        ObservationSource::HkcuBags,
        &mut snapshot.bags,
        "",
        0,
        &mut budget,
    );
}

fn inspect_streams(reader: &WindowsRegistryReader, snapshot: &mut InspectionSnapshot) {
    use winview_core::registry::keys::paths::{DESKTOP, STREAMS, STREAMS_DEFAULTS};

    let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);

    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        STREAMS,
        ObservationSource::HkcuStreams,
        &mut snapshot.streams,
        "",
        0,
        &mut budget,
    );
    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        STREAMS_DEFAULTS,
        ObservationSource::HkcuStreams,
        &mut snapshot.streams,
        "",
        0,
        &mut budget,
    );
    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        DESKTOP,
        ObservationSource::HkcuStreams,
        &mut snapshot.streams,
        "",
        0,
        &mut budget,
    );
}

fn inspect_advanced(
    reader: &dyn RegistryReader,
    snapshot: &mut InspectionSnapshot,
    max_value_bytes: usize,
    snapshot_budget: &mut SnapshotBudget,
) {
    use winview_core::registry::keys::advanced_values::*;
    use winview_core::registry::keys::paths::ADVANCED;

    let key_result = reader.open_key(RootKey::Hkcu, ADVANCED);
    let key = match key_result {
        Ok(k) => k,
        Err(e) => {
            snapshot.advanced.read_errors.push(InspectionError {
                key_path: ADVANCED.to_string(),
                root: RootKey::Hkcu,
                operation: "open_key".to_string(),
                error: e.to_string(),
            });
            return;
        }
    };

    let advanced_values = [
        HIDE_FILE_EXT,
        USE_COMPACT_MODE,
        AUTO_CHECK_SELECT,
        HIDDEN,
        LAUNCH_TO,
        FULL_ROW_SELECT,
        NO_STR_CMP_LOGICAL,
    ];

    for value_name in advanced_values {
        let value_result = key.get_value(value_name);
        let read_result = match value_result {
            Ok((tn, raw_bytes, decoded, status)) => {
                if let Some(error) = check_value_size(
                    RootKey::Hkcu,
                    ADVANCED,
                    value_name,
                    raw_bytes.len(),
                    max_value_bytes,
                ) {
                    snapshot.advanced.read_errors.push(error);
                    continue;
                }

                if let Some(error) = check_snapshot_budget(
                    snapshot_budget,
                    RootKey::Hkcu,
                    ADVANCED,
                    value_name,
                    raw_bytes.len(),
                ) {
                    snapshot.advanced.read_errors.push(error);
                    continue;
                }

                let decoded_opt = match status {
                    DecodingStatus::Decoded => Some(decoded),
                    _ => None,
                };
                ValueReadResult::Success(RawRegistryValue {
                    key_path: ADVANCED.to_string(),
                    value_name: value_name.to_string(),
                    root: RootKey::Hkcu,
                    raw_type_id: tn.to_raw_id(),
                    raw_bytes,
                    decoded: decoded_opt,
                    decoding_status: status,
                    source: ObservationSource::HkcuAdvanced,
                })
            }
            Err(e) => ValueReadResult::Failure(ReadError::Other(e.to_string())),
        };
        snapshot
            .advanced
            .values
            .insert(value_name.to_string(), read_result);
    }
}

fn inspect_clsid(reader: &WindowsRegistryReader, snapshot: &mut InspectionSnapshot) {
    let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
    use winview_core::registry::keys::paths::*;

    let clsid_paths = [
        (CLASSIC_CONTEXT_MENU_1, "CLASSIC_CONTEXT_MENU_1"),
        (CLASSIC_CONTEXT_MENU_2, "CLASSIC_CONTEXT_MENU_2"),
        (EXPLORER_START, "EXPLORER_START"),
        (EXPLORER_START_CMD, "EXPLORER_START_CMD"),
        (WIN10_EXPLORER_1, "WIN10_EXPLORER_1"),
        (WIN10_EXPLORER_2, "WIN10_EXPLORER_2"),
        (HOME_NAV, "HOME_NAV"),
        (GALLERY_NAV, "GALLERY_NAV"),
        (CLASSIC_SEARCH, "CLASSIC_SEARCH"),
        (COPY_TO, "COPY_TO"),
        (MOVE_TO, "MOVE_TO"),
    ];

    for (path, _name) in clsid_paths {
        inspect_key_tree(
            reader,
            RootKey::Hkcu,
            path,
            ObservationSource::HkcuClsid,
            &mut snapshot.clsid,
            "",
            0,
            &mut budget,
        );
    }
}

fn inspect_policies(reader: &WindowsRegistryReader, snapshot: &mut InspectionSnapshot) {
    use winview_core::registry::keys::paths::*;

    let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);

    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        EXPLORER_POLICIES,
        ObservationSource::HkcuPolicies,
        &mut snapshot.policies,
        "",
        0,
        &mut budget,
    );
    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        USER_EXPLORER_POLICIES,
        ObservationSource::HkcuPolicies,
        &mut snapshot.policies,
        "",
        0,
        &mut budget,
    );
    inspect_key_tree(
        reader,
        RootKey::Hklm,
        EXPLORER_POLICIES,
        ObservationSource::HklmPolicies,
        &mut snapshot.policies,
        "HKLM",
        0,
        &mut budget,
    );
    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        USER_COMDLG_PLACESBAR,
        ObservationSource::HkcuPolicies,
        &mut snapshot.policies,
        "",
        0,
        &mut budget,
    );
}

fn check_value_size(
    root: RootKey,
    path: &str,
    value_name: &str,
    actual_bytes: usize,
    max_bytes: usize,
) -> Option<InspectionError> {
    if actual_bytes <= max_bytes {
        return None;
    }

    Some(InspectionError {
        key_path: path.to_string(),
        root,
        operation: "value_size_limit".to_string(),
        error: format!(
            "Registry value {:?} contains {} bytes, exceeding the {} byte limit",
            value_name, actual_bytes, max_bytes
        ),
    })
}

fn check_snapshot_budget(
    budget: &mut SnapshotBudget,
    root: RootKey,
    path: &str,
    value_name: &str,
    actual_bytes: usize,
) -> Option<InspectionError> {
    if budget.try_capture(actual_bytes) {
        return None;
    }

    Some(InspectionError {
        key_path: path.to_string(),
        root,
        operation: "snapshot_size_limit".to_string(),
        error: format!(
            "Registry value {:?} contains {} bytes; \
             captured data is already {} of {} bytes",
            value_name, actual_bytes, budget.captured_bytes, budget.max_bytes
        ),
    })
}

#[allow(clippy::too_many_arguments)]
fn inspect_key_tree(
    reader: &dyn RegistryReader,
    root: RootKey,
    path: &str,
    source: ObservationSource,
    target: &mut BTreeMap<String, KeySnapshot>,
    key_prefix: &str,
    depth: usize,
    budget: &mut TraversalBudget,
) {
    let map_key = if key_prefix.is_empty() {
        path.to_string()
    } else {
        format!("{}\\{}", key_prefix, path)
    };

    if budget.limit_exceeded {
        return;
    }

    if depth >= MAX_DEPTH {
        let mut snapshot = KeySnapshot::default();
        snapshot.read_errors.push(InspectionError {
            key_path: path.to_string(),
            root,
            operation: "depth_limit".to_string(),
            error: format!("Maximum recursion depth ({}) exceeded", MAX_DEPTH),
        });
        target.insert(map_key, snapshot);
        return;
    }

    if budget.is_exhausted() {
        budget.limit_exceeded = true;

        let mut snapshot = KeySnapshot::default();
        snapshot.read_errors.push(InspectionError {
            key_path: path.to_string(),
            root,
            operation: "key_limit".to_string(),
            error: format!(
                "Maximum registry keys per domain ({}) exceeded",
                budget.max_keys
            ),
        });

        target.insert(map_key, snapshot);
        return;
    }

    budget.record_visit();

    let key_result = reader.open_key(root, path);
    let key = match key_result {
        Ok(k) => k,
        Err(e) => {
            let mut snapshot = KeySnapshot::default();
            snapshot.read_errors.push(InspectionError {
                key_path: path.to_string(),
                root,
                operation: "open_key".to_string(),
                error: e.to_string(),
            });
            target.insert(map_key.clone(), snapshot);
            return;
        }
    };

    let mut snapshot = KeySnapshot::default();

    // Read values
    let values_result = key.enum_values();
    if let Ok(values) = values_result {
        if values.len() > budget.max_values_per_key {
            snapshot.read_errors.push(InspectionError {
                key_path: path.to_string(),
                root,
                operation: "value_count_limit".to_string(),
                error: format!(
                    "Maximum registry values per key ({}) exceeded; \
                     {} values omitted",
                    budget.max_values_per_key,
                    values.len() - budget.max_values_per_key
                ),
            });
        }

        for (value_name, _type_name) in values.into_iter().take(budget.max_values_per_key) {
            let value_result = key.get_value(&value_name);
            let read_result = match value_result {
                Ok((tn, raw_bytes, decoded, status)) => {
                    if let Some(error) = check_value_size(
                        root,
                        path,
                        &value_name,
                        raw_bytes.len(),
                        budget.max_value_bytes,
                    ) {
                        snapshot.read_errors.push(error);
                        continue;
                    }

                    let decoded_opt = match status {
                        DecodingStatus::Decoded => Some(decoded),
                        _ => None,
                    };
                    ValueReadResult::Success(RawRegistryValue {
                        key_path: path.to_string(),
                        value_name: value_name.clone(),
                        root,
                        raw_type_id: tn.to_raw_id(),
                        raw_bytes,
                        decoded: decoded_opt,
                        decoding_status: status,
                        source,
                    })
                }
                Err(e) => ValueReadResult::Failure(ReadError::Other(e.to_string())),
            };
            snapshot.values.insert(value_name, read_result);
        }
    }

    // Recursively read subkeys (with depth limit)
    let subkeys_result = key.enum_subkeys();
    if let Ok(subkeys) = subkeys_result {
        for subkey_name in subkeys {
            let subkey_path = format!(r"{}\{}", path, subkey_name);
            inspect_key_tree(
                reader,
                root,
                &subkey_path,
                source,
                &mut snapshot.subkeys,
                "",
                depth + 1,
                budget,
            );
            if budget.limit_exceeded {
                break;
            }
        }
    }

    target.insert(map_key.clone(), snapshot);
}
fn has_depth_limit_error(key: &KeySnapshot) -> bool {
    key.read_errors.iter().any(|e| e.operation == "depth_limit")
        || key.subkeys.values().any(has_depth_limit_error)
}

fn has_key_limit_error(key: &KeySnapshot) -> bool {
    key.read_errors.iter().any(|e| e.operation == "key_limit")
        || key.subkeys.values().any(has_key_limit_error)
}

fn snapshot_has_depth_limit(snapshot: &InspectionSnapshot) -> bool {
    [
        &snapshot.bags,
        &snapshot.streams,
        &snapshot.clsid,
        &snapshot.policies,
    ]
    .iter()
    .any(|map| map.values().any(has_depth_limit_error))
}

fn snapshot_has_key_limit(snapshot: &InspectionSnapshot) -> bool {
    [
        &snapshot.bags,
        &snapshot.streams,
        &snapshot.clsid,
        &snapshot.policies,
    ]
    .iter()
    .any(|map| map.values().any(has_key_limit_error))
}

fn has_value_count_limit_error(key: &KeySnapshot) -> bool {
    key.read_errors
        .iter()
        .any(|e| e.operation == "value_count_limit")
        || key.subkeys.values().any(has_value_count_limit_error)
}

fn snapshot_has_value_count_limit(snapshot: &InspectionSnapshot) -> bool {
    [
        &snapshot.bags,
        &snapshot.streams,
        &snapshot.clsid,
        &snapshot.policies,
    ]
    .iter()
    .any(|map| map.values().any(has_value_count_limit_error))
}

fn has_value_size_limit_error(key: &KeySnapshot) -> bool {
    key.read_errors
        .iter()
        .any(|e| e.operation == "value_size_limit")
        || key.subkeys.values().any(has_value_size_limit_error)
}

fn snapshot_has_value_size_limit(snapshot: &InspectionSnapshot) -> bool {
    snapshot
        .hkcu_folder_types
        .read_errors
        .iter()
        .any(|e| e.operation == "value_size_limit")
        || snapshot
            .hklm_folder_types
            .read_errors
            .iter()
            .any(|e| e.operation == "value_size_limit")
        || has_value_size_limit_error(&snapshot.advanced)
        || [
            &snapshot.bags,
            &snapshot.streams,
            &snapshot.clsid,
            &snapshot.policies,
        ]
        .iter()
        .any(|map| map.values().any(has_value_size_limit_error))
}

fn snapshot_has_snapshot_size_limit(snapshot: &InspectionSnapshot) -> bool {
    snapshot
        .advanced
        .read_errors
        .iter()
        .any(|e| e.operation == "snapshot_size_limit")
        || snapshot
            .hkcu_folder_types
            .read_errors
            .iter()
            .any(|e| e.operation == "snapshot_size_limit")
        || snapshot
            .hklm_folder_types
            .read_errors
            .iter()
            .any(|e| e.operation == "snapshot_size_limit")
}

fn finalize_inspection_status(snapshot: &mut InspectionSnapshot) {
    let total_errors = snapshot.errors.len()
        + snapshot.hkcu_folder_types.read_errors.len()
        + snapshot.hklm_folder_types.read_errors.len()
        + snapshot.advanced.read_errors.len();
    let mut total = total_errors;
    for ks in snapshot.bags.values() {
        total += ks.read_errors.len();
    }
    for ks in snapshot.streams.values() {
        total += ks.read_errors.len();
    }
    for ks in snapshot.clsid.values() {
        total += ks.read_errors.len();
    }
    for ks in snapshot.policies.values() {
        total += ks.read_errors.len();
    }

    let has_data = !snapshot.hkcu_folder_types.types.is_empty()
        || !snapshot.hklm_folder_types.types.is_empty()
        || !snapshot.bags.is_empty()
        || !snapshot.streams.is_empty()
        || !snapshot.advanced.values.is_empty()
        || !snapshot.clsid.is_empty()
        || !snapshot.policies.is_empty();

    if snapshot_has_depth_limit(snapshot) {
        snapshot.status = InspectionStatus::Partial {
            reason: "Maximum recursion depth exceeded".to_string(),
        };
    } else if snapshot_has_key_limit(snapshot) {
        snapshot.status = InspectionStatus::Partial {
            reason: "Maximum registry key count exceeded".to_string(),
        };
    } else if snapshot_has_value_count_limit(snapshot) {
        snapshot.status = InspectionStatus::Partial {
            reason: "Maximum registry values per key exceeded".to_string(),
        };
    } else if snapshot_has_value_size_limit(snapshot) {
        snapshot.status = InspectionStatus::Partial {
            reason: "Maximum individual registry value size exceeded".to_string(),
        };
    } else if snapshot_has_snapshot_size_limit(snapshot) {
        snapshot.status = InspectionStatus::Partial {
            reason: "Maximum captured snapshot size exceeded".to_string(),
        };
    } else if total == 0 {
        snapshot.status = InspectionStatus::Complete;
    } else if has_data {
        snapshot.status = InspectionStatus::Partial {
            reason: format!("{} read errors encountered", total),
        };
    } else {
        snapshot.status = InspectionStatus::Failed {
            error: format!("Inspection failed with {} errors", total),
        };
    }
}

#[cfg(test)]
mod depth_tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use winview_core::model::{InspectionSnapshot, KeySnapshot, ObservationSource};

    type FakeValue = (u32, Vec<u8>, Option<RegistryValue>, DecodingStatus);

    type FakeValues = BTreeMap<String, BTreeMap<String, FakeValue>>;

    struct FakeReader {
        data: Arc<BTreeMap<String, Vec<String>>>,
        opened_keys: Mutex<Vec<(RootKey, String)>>,
        values: Arc<FakeValues>,
    }

    impl FakeReader {
        fn new(data: BTreeMap<String, Vec<String>>) -> Self {
            Self {
                data: Arc::new(data),
                opened_keys: Mutex::new(Vec::new()),
                values: Arc::new(FakeValues::new()),
            }
        }

        fn with_values(data: BTreeMap<String, Vec<String>>, values: FakeValues) -> Self {
            Self {
                data: Arc::new(data),
                opened_keys: Mutex::new(Vec::new()),
                values: Arc::new(values),
            }
        }
    }

    struct FakeKey {
        path: String,
        data: Arc<BTreeMap<String, Vec<String>>>,
        values: Arc<FakeValues>,
    }

    impl RegistryKey for FakeKey {
        fn enum_values(&self) -> RegistryResult<Vec<(String, ValueTypeName)>> {
            if let Some(vals) = self.values.get(&self.path) {
                Ok(vals
                    .iter()
                    .map(|(name, (type_id, _, _, _))| (name.clone(), type_id_to_name(*type_id)))
                    .collect())
            } else {
                Ok(vec![("TestValue".to_string(), ValueTypeName::RegDword)])
            }
        }
        fn enum_subkeys(&self) -> RegistryResult<Vec<String>> {
            Ok(self.data.get(&self.path).cloned().unwrap_or_default())
        }
        fn get_value(
            &self,
            name: &str,
        ) -> RegistryResult<(ValueTypeName, Vec<u8>, RegistryValue, DecodingStatus)> {
            if let Some(vals) = self.values.get(&self.path) {
                if let Some((type_id, raw_bytes, decoded, status)) = vals.get(name) {
                    let decoded = decoded.clone().ok_or(RegistryError::InvalidData(
                        "no decoded value in fake".into(),
                    ))?;
                    return Ok((
                        type_id_to_name(*type_id),
                        raw_bytes.clone(),
                        decoded,
                        status.clone(),
                    ));
                }
            }

            if name == "TestValue" {
                Ok((
                    ValueTypeName::RegDword,
                    42u32.to_le_bytes().to_vec(),
                    RegistryValue::Dword(42),
                    DecodingStatus::Decoded,
                ))
            } else {
                Err(RegistryError::ValueNotFound {
                    root: RootKey::Hkcu,
                    path: "".into(),
                    name: "".into(),
                })
            }
        }
        fn get_value_type(&self, name: &str) -> RegistryResult<ValueTypeName> {
            if let Some(vals) = self.values.get(&self.path) {
                if let Some((type_id, _, _, _)) = vals.get(name) {
                    return Ok(type_id_to_name(*type_id));
                }
            }
            if name == "TestValue" {
                Ok(ValueTypeName::RegDword)
            } else {
                Ok(ValueTypeName::RegSz)
            }
        }
        fn open_subkey(&self, name: &str) -> RegistryResult<Box<dyn RegistryKey>> {
            let subpath = format!("{}\\{}", self.path, name);
            if self.data.contains_key(&subpath) {
                Ok(Box::new(FakeKey {
                    path: subpath,
                    data: self.data.clone(),
                    values: self.values.clone(),
                }))
            } else {
                Err(RegistryError::KeyNotFound {
                    root: RootKey::Hkcu,
                    path: subpath,
                })
            }
        }
    }

    impl RegistryReader for FakeReader {
        fn open_key(&self, root: RootKey, path: &str) -> RegistryResult<Box<dyn RegistryKey>> {
            self.opened_keys
                .lock()
                .unwrap()
                .push((root, path.to_string()));

            if self.data.contains_key(path) {
                Ok(Box::new(FakeKey {
                    path: path.to_string(),
                    data: self.data.clone(),
                    values: self.values.clone(),
                }))
            } else {
                Err(RegistryError::KeyNotFound {
                    root: RootKey::Hkcu,
                    path: path.to_string(),
                })
            }
        }
        fn enum_keys(&self, _root: RootKey, _path: &str) -> RegistryResult<Vec<String>> {
            Ok(vec![])
        }
        fn get_value(
            &self,
            _root: RootKey,
            _path: &str,
            _name: &str,
        ) -> RegistryResult<(ValueTypeName, Vec<u8>, RegistryValue, DecodingStatus)> {
            Err(RegistryError::ValueNotFound {
                root: RootKey::Hkcu,
                path: "".into(),
                name: "".into(),
            })
        }
        fn get_value_type(
            &self,
            _root: RootKey,
            _path: &str,
            _name: &str,
        ) -> RegistryResult<ValueTypeName> {
            Ok(ValueTypeName::RegSz)
        }
        fn inspect_all(&self) -> RegistryResult<InspectionSnapshot> {
            Ok(InspectionSnapshot::default())
        }
    }

    fn build_hierarchy(depth: usize) -> FakeReader {
        let mut data = BTreeMap::new();
        let mut path = "root".to_string();
        for i in 0..=depth {
            let subkeys = if i < depth {
                vec!["child".to_string()]
            } else {
                vec![]
            };
            data.insert(path.clone(), subkeys);
            path = format!("{}\\child", path);
        }
        FakeReader::new(data)
    }

    fn has_depth_limit(ks: &KeySnapshot) -> bool {
        ks.read_errors.iter().any(|e| e.operation == "depth_limit")
            || ks.subkeys.values().any(has_depth_limit)
    }

    fn type_id_to_name(id: u32) -> ValueTypeName {
        match id {
            1 => ValueTypeName::RegSz,
            2 => ValueTypeName::RegExpandSz,
            3 => ValueTypeName::RegBinary,
            4 => ValueTypeName::RegDword,
            7 => ValueTypeName::RegMultiSz,
            11 => ValueTypeName::RegQword,
            _ => ValueTypeName::Other(id),
        }
    }

    const TEST_FOLDER_TYPES: &str = "FolderTypes";

    const TEST_FOLDER_GUID: &str = "{11111111-1111-1111-1111-111111111111}";

    const TEST_VIEW_GUID: &str = "{00000000-0000-0000-0000-000000000000}";

    fn build_folder_types_fixture(
        canonical_name: Option<&str>,
        include_col_info: bool,
    ) -> FakeReader {
        let guid_path = format!(r"{}\{}", TEST_FOLDER_TYPES, TEST_FOLDER_GUID);

        let view_path = format!(r"{}\TopViews\{}", guid_path, TEST_VIEW_GUID);

        let mut data = BTreeMap::new();

        data.insert(
            TEST_FOLDER_TYPES.to_string(),
            vec![TEST_FOLDER_GUID.to_string()],
        );

        data.insert(guid_path.clone(), Vec::new());
        data.insert(view_path.clone(), Vec::new());

        let mut values = FakeValues::new();

        if let Some(name) = canonical_name {
            let raw_bytes: Vec<u8> = name
                .encode_utf16()
                .chain(std::iter::once(0))
                .flat_map(|unit| unit.to_le_bytes())
                .collect();

            values.entry(guid_path).or_default().insert(
                "CanonicalName".to_string(),
                (
                    1,
                    raw_bytes,
                    Some(RegistryValue::String(name.to_string())),
                    DecodingStatus::Decoded,
                ),
            );
        }

        let mut view_values = BTreeMap::new();

        view_values.insert(
            "Mode".to_string(),
            (
                4,
                42u32.to_le_bytes().to_vec(),
                Some(RegistryValue::Dword(42)),
                DecodingStatus::Decoded,
            ),
        );

        if include_col_info {
            let bytes = vec![1u8; 8];

            view_values.insert(
                "ColInfo".to_string(),
                (
                    3,
                    bytes.clone(),
                    Some(RegistryValue::Binary(bytes)),
                    DecodingStatus::Decoded,
                ),
            );
        }

        values.insert(view_path, view_values);

        FakeReader::with_values(data, values)
    }

    fn inspect_test_tree(
        reader: &FakeReader,
        max_keys: usize,
    ) -> (InspectionSnapshot, TraversalBudget) {
        let mut snapshot = InspectionSnapshot::default();
        let mut budget = TraversalBudget::new(max_keys);

        inspect_key_tree(
            reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            0,
            &mut budget,
        );

        finalize_inspection_status(&mut snapshot);

        (snapshot, budget)
    }

    #[test]
    fn depth_below_limit_no_error() {
        let reader = build_hierarchy(30);
        let mut target = BTreeMap::new();
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut target,
            "",
            0,
            &mut budget,
        );
        assert!(target.values().all(|ks| !has_depth_limit(ks)));
    }

    #[test]
    fn depth_at_boundary_no_error() {
        let reader = build_hierarchy(31);
        let mut target = BTreeMap::new();
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut target,
            "",
            0,
            &mut budget,
        );
        assert!(target.values().all(|ks| !has_depth_limit(ks)));
    }

    #[test]
    fn depth_beyond_limit_blocked() {
        let reader = build_hierarchy(32);
        let mut target = BTreeMap::new();
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut target,
            "",
            0,
            &mut budget,
        );
        assert!(target.values().any(has_depth_limit));

        let opened = reader.opened_keys.lock().unwrap();
        assert_eq!(opened.len(), 32);

        // The depth-32 path must never have been opened.
        let depth_32_path =
            (0..MAX_DEPTH).fold("root".to_string(), |acc, _| format!("{}\\child", acc));
        assert!(!opened.iter().any(|(_, p)| p == &depth_32_path));
    }

    #[test]
    fn depth_limit_error_has_correct_root_and_path() {
        let reader = build_hierarchy(32);
        let mut target = BTreeMap::new();
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut target,
            "",
            0,
            &mut budget,
        );
        fn find_err(ks: &KeySnapshot) -> Option<&InspectionError> {
            ks.read_errors
                .iter()
                .find(|e| e.operation == "depth_limit")
                .or_else(|| ks.subkeys.values().find_map(find_err))
        }
        let err = target
            .values()
            .find_map(find_err)
            .expect("depth_limit error not found");
        assert_eq!(err.root, RootKey::Hkcu);
        assert!(err.key_path.contains("child"));
    }

    #[test]
    fn depth_limit_preserves_parent_data() {
        let reader = build_hierarchy(32);
        let mut target = BTreeMap::new();
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut target,
            "",
            0,
            &mut budget,
        );
        assert!(!target.is_empty());

        let root = target.get("root").expect("root snapshot missing");
        let value = root.values.get("TestValue").expect("TestValue missing");
        match value {
            ValueReadResult::Success(r) => {
                assert_eq!(r.raw_bytes, vec![42u8, 0, 0, 0]);
                assert!(matches!(r.decoded.as_ref(), Some(RegistryValue::Dword(42))));
            }
            _ => panic!("expected successful value read"),
        }
    }

    #[test]
    fn depth_limit_produces_partial_status() {
        let mut snapshot = InspectionSnapshot::default();
        let mut ks = KeySnapshot::default();
        ks.read_errors.push(InspectionError {
            key_path: "test".to_string(),
            root: RootKey::Hkcu,
            operation: "depth_limit".to_string(),
            error: "Maximum recursion depth (32) exceeded".to_string(),
        });
        snapshot.bags.insert("test".to_string(), ks);
        finalize_inspection_status(&mut snapshot);
        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn no_depth_limit_stays_complete() {
        let mut snapshot = InspectionSnapshot::default();
        finalize_inspection_status(&mut snapshot);
        assert!(matches!(snapshot.status, InspectionStatus::Complete));
    }

    #[test]
    fn traversal_budget_starts_available() {
        let budget = TraversalBudget::new(2);

        assert_eq!(budget.visited_keys, 0);
        assert!(!budget.is_exhausted());
        assert!(!budget.limit_exceeded);
    }

    #[test]
    fn traversal_budget_reaches_capacity() {
        let mut budget = TraversalBudget::new(2);

        budget.record_visit();
        assert!(!budget.is_exhausted());

        budget.record_visit();
        assert!(budget.is_exhausted());
        assert!(!budget.limit_exceeded);
    }

    #[test]
    fn key_count_exceeded_blocks_extra_key() {
        let reader = build_hierarchy(3);
        let (snapshot, budget) = inspect_test_tree(&reader, 3);

        let opened = reader.opened_keys.lock().unwrap();

        assert_eq!(
            opened.as_slice(),
            &[
                (RootKey::Hkcu, "root".to_string()),
                (RootKey::Hkcu, r"root\child".to_string()),
                (RootKey::Hkcu, r"root\child\child".to_string()),
            ]
        );

        assert_eq!(budget.visited_keys, 3);
        assert!(budget.limit_exceeded);
        assert!(snapshot_has_key_limit(&snapshot));

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn key_count_below_limit() {
        let reader = build_hierarchy(2);
        let (snapshot, budget) = inspect_test_tree(&reader, 4);

        let opened = reader.opened_keys.lock().unwrap();

        assert_eq!(
            opened.as_slice(),
            &[
                (RootKey::Hkcu, "root".to_string()),
                (RootKey::Hkcu, r"root\child".to_string()),
                (RootKey::Hkcu, r"root\child\child".to_string()),
            ]
        );

        assert_eq!(budget.visited_keys, 3);
        assert!(!budget.limit_exceeded);
        assert!(!snapshot_has_key_limit(&snapshot));
        assert!(matches!(snapshot.status, InspectionStatus::Complete));
    }

    #[test]
    fn key_count_exact_limit() {
        let reader = build_hierarchy(2);
        let (snapshot, budget) = inspect_test_tree(&reader, 3);

        let opened = reader.opened_keys.lock().unwrap();

        assert_eq!(
            opened.as_slice(),
            &[
                (RootKey::Hkcu, "root".to_string()),
                (RootKey::Hkcu, r"root\child".to_string()),
                (RootKey::Hkcu, r"root\child\child".to_string()),
            ]
        );

        assert_eq!(budget.visited_keys, 3);
        assert!(!budget.limit_exceeded);
        assert!(!snapshot_has_key_limit(&snapshot));
        assert!(matches!(snapshot.status, InspectionStatus::Complete));
    }

    #[test]
    fn key_limit_preserves_values() {
        let reader = build_hierarchy(3);
        let (snapshot, _budget) = inspect_test_tree(&reader, 3);

        let root = snapshot.bags.get("root").expect("root missing");
        let child = root.subkeys.get(r"root\child").expect("child missing");
        let grandchild = root
            .subkeys
            .get(r"root\child")
            .and_then(|k| k.subkeys.get(r"root\child\child"))
            .expect("grandchild missing");

        for ks in [root, child, grandchild] {
            let value = ks.values.get("TestValue").expect("TestValue missing");
            match value {
                ValueReadResult::Success(r) => {
                    assert_eq!(r.raw_bytes, vec![42u8, 0, 0, 0]);
                    assert!(matches!(r.decoded.as_ref(), Some(RegistryValue::Dword(42))));
                }
                _ => panic!("expected successful value read"),
            }
        }
    }

    #[test]
    fn key_limit_error_has_correct_location() {
        let reader = build_hierarchy(3);
        let (snapshot, _budget) = inspect_test_tree(&reader, 3);

        let expected_path = r"root\child\child\child".to_string();

        fn find_key_limit(ks: &KeySnapshot) -> Option<&InspectionError> {
            ks.read_errors
                .iter()
                .find(|e| e.operation == "key_limit")
                .or_else(|| ks.subkeys.values().find_map(find_key_limit))
        }

        let err = snapshot
            .bags
            .values()
            .find_map(find_key_limit)
            .expect("key_limit error not found");

        assert_eq!(err.root, RootKey::Hkcu);
        assert_eq!(err.key_path, expected_path);

        // Exactly one key_limit error across the whole tree.
        fn count_key_limit(ks: &KeySnapshot) -> usize {
            ks.read_errors
                .iter()
                .filter(|e| e.operation == "key_limit")
                .count()
                + ks.subkeys.values().map(count_key_limit).sum::<usize>()
        }
        let count = snapshot.bags.values().map(count_key_limit).sum::<usize>();
        assert_eq!(count, 1);
    }

    #[test]
    fn key_limit_stops_sibling_traversal() {
        let mut data = BTreeMap::new();
        let children: Vec<String> = (0..5).map(|i| format!("child{}", i)).collect();
        for child in &children {
            data.insert(format!(r"root\{}", child), Vec::new());
        }
        data.insert("root".to_string(), children);

        let reader = FakeReader::new(data);
        let (snapshot, budget) = inspect_test_tree(&reader, 3);

        let opened = reader.opened_keys.lock().unwrap();

        assert_eq!(
            opened.as_slice(),
            &[
                (RootKey::Hkcu, "root".to_string()),
                (RootKey::Hkcu, r"root\child0".to_string()),
                (RootKey::Hkcu, r"root\child1".to_string()),
            ]
        );

        assert_eq!(budget.visited_keys, 3);
        assert!(budget.limit_exceeded);
        assert!(snapshot_has_key_limit(&snapshot));

        // Exactly one key_limit error, recorded on the blocked third child.
        fn count_key_limit(ks: &KeySnapshot) -> usize {
            ks.read_errors
                .iter()
                .filter(|e| e.operation == "key_limit")
                .count()
                + ks.subkeys.values().map(count_key_limit).sum::<usize>()
        }
        let count = snapshot.bags.values().map(count_key_limit).sum::<usize>();
        assert_eq!(count, 1);
    }

    #[test]
    fn key_budget_shared_across_roots() {
        let mut data = BTreeMap::new();
        data.insert("root1".to_string(), vec!["child".to_string()]);
        data.insert(r"root1\child".to_string(), vec![]);
        data.insert("root2".to_string(), vec!["child".to_string()]);
        data.insert(r"root2\child".to_string(), vec![]);

        let reader = FakeReader::new(data);

        let mut snapshot = InspectionSnapshot::default();
        let mut budget = TraversalBudget::new(3);

        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root1",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            0,
            &mut budget,
        );
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root2",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            0,
            &mut budget,
        );

        let opened = reader.opened_keys.lock().unwrap();

        // Budget of 3: root1, root1\child, root2 — then limit exceeded.
        assert_eq!(
            opened.as_slice(),
            &[
                (RootKey::Hkcu, "root1".to_string()),
                (RootKey::Hkcu, r"root1\child".to_string()),
                (RootKey::Hkcu, "root2".to_string()),
            ]
        );

        assert_eq!(budget.visited_keys, 3);
        assert!(budget.limit_exceeded);
        assert!(snapshot_has_key_limit(&snapshot));
    }

    #[test]
    fn value_count_below_limit() {
        let reader = build_hierarchy(0);
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        budget.max_values_per_key = 2;

        let mut snapshot = InspectionSnapshot::default();
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            0,
            &mut budget,
        );

        let root = snapshot.bags.get("root").expect("root missing");
        assert!(root.values.contains_key("TestValue"));
        assert!(!has_value_count_limit_error(root));
        assert!(!snapshot_has_value_count_limit(&snapshot));
    }

    #[test]
    fn value_count_exact_limit() {
        let reader = build_hierarchy(0);
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        budget.max_values_per_key = 1;

        let mut snapshot = InspectionSnapshot::default();
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            0,
            &mut budget,
        );
        finalize_inspection_status(&mut snapshot);

        let root = snapshot.bags.get("root").expect("root missing");
        assert!(root.values.contains_key("TestValue"));
        assert!(!has_value_count_limit_error(root));
        assert!(!snapshot_has_value_count_limit(&snapshot));
        assert!(matches!(snapshot.status, InspectionStatus::Complete));
    }

    #[test]
    fn value_count_exceeded() {
        let reader = build_hierarchy(0);
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        budget.max_values_per_key = 0;

        let mut snapshot = InspectionSnapshot::default();
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            0,
            &mut budget,
        );
        finalize_inspection_status(&mut snapshot);

        let root = snapshot.bags.get("root").expect("root missing");
        assert!(!root.values.contains_key("TestValue"));
        assert!(has_value_count_limit_error(root));
        assert!(snapshot_has_value_count_limit(&snapshot));
        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn value_count_limit_does_not_stop_child_traversal() {
        let reader = build_hierarchy(1);
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        budget.max_values_per_key = 0;

        let mut snapshot = InspectionSnapshot::default();
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            0,
            &mut budget,
        );

        let root = snapshot.bags.get("root").expect("root missing");
        let child = root.subkeys.get(r"root\child").expect("child missing");

        assert!(has_value_count_limit_error(root));
        assert!(has_value_count_limit_error(child));
        assert!(!budget.limit_exceeded);
    }

    #[test]
    fn value_size_above_limit_is_omitted() {
        let reader = build_hierarchy(0);
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        budget.max_value_bytes = 3;

        let mut snapshot = InspectionSnapshot::default();

        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            0,
            &mut budget,
        );

        finalize_inspection_status(&mut snapshot);

        let root = snapshot.bags.get("root").expect("root missing");

        assert!(!root.values.contains_key("TestValue"));
        assert!(has_value_size_limit_error(root));
        assert!(!budget.limit_exceeded);

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));

        let err = root
            .read_errors
            .iter()
            .find(|e| e.operation == "value_size_limit")
            .expect("value size error missing");

        assert_eq!(err.root, RootKey::Hkcu);
        assert_eq!(err.key_path, "root");
        assert!(err.error.contains("TestValue"));
    }

    #[test]
    fn value_size_below_limit_is_retained() {
        let reader = build_hierarchy(0);
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        budget.max_value_bytes = 5;

        let mut snapshot = InspectionSnapshot::default();

        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            0,
            &mut budget,
        );

        finalize_inspection_status(&mut snapshot);

        let root = snapshot.bags.get("root").expect("root missing");
        let value = root.values.get("TestValue").expect("TestValue missing");

        match value {
            ValueReadResult::Success(r) => {
                assert_eq!(r.raw_bytes, vec![42u8, 0, 0, 0]);
                assert!(matches!(r.decoded.as_ref(), Some(RegistryValue::Dword(42))));
            }
            _ => panic!("expected successful value read"),
        }

        assert!(!has_value_size_limit_error(root));
        assert!(!snapshot_has_value_size_limit(&snapshot));
        assert!(matches!(snapshot.status, InspectionStatus::Complete));
    }

    #[test]
    fn value_size_exact_limit_is_retained() {
        let reader = build_hierarchy(0);
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        budget.max_value_bytes = 4;

        let mut snapshot = InspectionSnapshot::default();

        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            0,
            &mut budget,
        );

        finalize_inspection_status(&mut snapshot);

        let root = snapshot.bags.get("root").expect("root missing");
        assert!(root.values.contains_key("TestValue"));
        assert!(!has_value_size_limit_error(root));
        assert!(!snapshot_has_value_size_limit(&snapshot));
        assert!(matches!(snapshot.status, InspectionStatus::Complete));
    }

    #[test]
    fn value_size_limit_does_not_stop_child_traversal() {
        let reader = build_hierarchy(1);
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        budget.max_value_bytes = 3;

        let mut snapshot = InspectionSnapshot::default();

        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            0,
            &mut budget,
        );

        let root = snapshot.bags.get("root").expect("root missing");
        let child = root.subkeys.get(r"root\child").expect("child missing");

        assert!(has_value_size_limit_error(root));
        assert!(has_value_size_limit_error(child));
        assert!(!budget.limit_exceeded);
    }

    #[test]
    fn advanced_value_size_exact_limit() {
        use winview_core::registry::keys::advanced_values::HIDE_FILE_EXT;
        use winview_core::registry::keys::paths::ADVANCED;
        use winview_core::test_support::FakeRegistryReader;

        let reader = FakeRegistryReader::new();
        reader.insert_value(
            RootKey::Hkcu,
            ADVANCED,
            HIDE_FILE_EXT,
            4,
            42u32.to_le_bytes().to_vec(),
            Some(RegistryValue::Dword(42)),
        );

        let mut snapshot = InspectionSnapshot::default();
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);
        inspect_advanced(&reader, &mut snapshot, 4, &mut snapshot_budget);
        finalize_inspection_status(&mut snapshot);

        let value = snapshot
            .advanced
            .values
            .get(HIDE_FILE_EXT)
            .expect("HideFileExt missing");

        match value {
            ValueReadResult::Success(r) => {
                assert_eq!(r.raw_bytes, vec![42u8, 0, 0, 0]);
                assert!(matches!(r.decoded.as_ref(), Some(RegistryValue::Dword(42))));
            }
            _ => panic!("expected successful value read"),
        }

        assert!(!has_value_size_limit_error(&snapshot.advanced));
        assert!(!snapshot_has_value_size_limit(&snapshot));
        assert!(matches!(snapshot.status, InspectionStatus::Complete));
    }

    #[test]
    fn advanced_oversized_value_is_omitted() {
        use winview_core::registry::keys::advanced_values::HIDE_FILE_EXT;
        use winview_core::registry::keys::paths::ADVANCED;
        use winview_core::test_support::FakeRegistryReader;

        let reader = FakeRegistryReader::new();
        reader.insert_value(
            RootKey::Hkcu,
            ADVANCED,
            HIDE_FILE_EXT,
            4,
            42u32.to_le_bytes().to_vec(),
            Some(RegistryValue::Dword(42)),
        );

        let mut snapshot = InspectionSnapshot::default();
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);
        inspect_advanced(&reader, &mut snapshot, 3, &mut snapshot_budget);
        finalize_inspection_status(&mut snapshot);

        assert!(!snapshot.advanced.values.contains_key(HIDE_FILE_EXT));
        assert!(has_value_size_limit_error(&snapshot.advanced));
        assert!(snapshot_has_value_size_limit(&snapshot));
        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));

        let err = snapshot
            .advanced
            .read_errors
            .iter()
            .find(|e| e.operation == "value_size_limit")
            .expect("value size error missing");

        assert_eq!(err.root, RootKey::Hkcu);
        assert_eq!(err.key_path, ADVANCED);
        assert!(err.error.contains(HIDE_FILE_EXT));
    }

    #[test]
    fn advanced_oversized_value_does_not_block_other_values() {
        use winview_core::registry::keys::advanced_values::{HIDE_FILE_EXT, USE_COMPACT_MODE};
        use winview_core::registry::keys::paths::ADVANCED;
        use winview_core::test_support::FakeRegistryReader;

        let reader = FakeRegistryReader::new();
        reader.insert_value(
            RootKey::Hkcu,
            ADVANCED,
            HIDE_FILE_EXT,
            11,
            1u64.to_le_bytes().to_vec(),
            Some(RegistryValue::Qword(1)),
        );
        reader.insert_value(
            RootKey::Hkcu,
            ADVANCED,
            USE_COMPACT_MODE,
            4,
            42u32.to_le_bytes().to_vec(),
            Some(RegistryValue::Dword(42)),
        );

        let mut snapshot = InspectionSnapshot::default();
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);
        inspect_advanced(&reader, &mut snapshot, 4, &mut snapshot_budget);
        finalize_inspection_status(&mut snapshot);

        assert!(!snapshot.advanced.values.contains_key(HIDE_FILE_EXT));

        let value = snapshot
            .advanced
            .values
            .get(USE_COMPACT_MODE)
            .expect("UseCompactMode missing");

        match value {
            ValueReadResult::Success(r) => {
                assert_eq!(r.raw_bytes, vec![42u8, 0, 0, 0]);
                assert!(matches!(r.decoded.as_ref(), Some(RegistryValue::Dword(42))));
            }
            _ => panic!("expected successful value read"),
        }

        let count = snapshot
            .advanced
            .read_errors
            .iter()
            .filter(|e| e.operation == "value_size_limit")
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn folder_type_value_size_exact_limit() {
        let reader = build_folder_types_fixture(None, false);

        let mut snapshot = InspectionSnapshot::default();
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        inspect_folder_types_hive(
            &reader,
            RootKey::Hkcu,
            TEST_FOLDER_TYPES,
            ObservationSource::HkcuFolderTypes,
            &mut snapshot.hkcu_folder_types,
            4,
            &mut snapshot_budget,
        );

        finalize_inspection_status(&mut snapshot);

        let folder = snapshot
            .hkcu_folder_types
            .types
            .get(TEST_FOLDER_GUID)
            .expect("folder type missing");

        let view = folder.top_views.get(TEST_VIEW_GUID).expect("view missing");

        assert_eq!(view.mode, Some(42));

        match view.raw_values.get("Mode") {
            Some(ValueReadResult::Success(raw)) => {
                assert_eq!(raw.raw_bytes, vec![42, 0, 0, 0]);
                assert_eq!(raw.decoded.as_ref(), Some(&RegistryValue::Dword(42)));
            }
            _ => panic!("expected successful Mode observation"),
        }

        assert!(!snapshot_has_value_size_limit(&snapshot));
    }

    #[test]
    fn folder_type_oversized_view_value_is_omitted() {
        let reader = build_folder_types_fixture(None, true);

        let mut snapshot = InspectionSnapshot::default();
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        inspect_folder_types_hive(
            &reader,
            RootKey::Hkcu,
            TEST_FOLDER_TYPES,
            ObservationSource::HkcuFolderTypes,
            &mut snapshot.hkcu_folder_types,
            4,
            &mut snapshot_budget,
        );

        finalize_inspection_status(&mut snapshot);

        let folder = snapshot
            .hkcu_folder_types
            .types
            .get(TEST_FOLDER_GUID)
            .expect("folder type missing");

        let view = folder.top_views.get(TEST_VIEW_GUID).expect("view missing");

        assert_eq!(view.mode, Some(42));
        assert!(!view.raw_values.contains_key("ColInfo"));
        assert_eq!(view.col_info, None);

        let errs: Vec<_> = snapshot
            .hkcu_folder_types
            .read_errors
            .iter()
            .filter(|e| e.operation == "value_size_limit")
            .collect();
        assert_eq!(errs.len(), 1);
        let err = &errs[0];
        assert_eq!(
            err.key_path,
            format!(
                r"{}\{}\TopViews\{}",
                TEST_FOLDER_TYPES, TEST_FOLDER_GUID, TEST_VIEW_GUID
            )
        );
        assert!(err.error.contains("ColInfo"));

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn folder_type_oversized_canonical_name_is_omitted() {
        let reader = build_folder_types_fixture(Some("GeneralItems"), false);

        let mut snapshot = InspectionSnapshot::default();
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        inspect_folder_types_hive(
            &reader,
            RootKey::Hkcu,
            TEST_FOLDER_TYPES,
            ObservationSource::HkcuFolderTypes,
            &mut snapshot.hkcu_folder_types,
            4,
            &mut snapshot_budget,
        );

        finalize_inspection_status(&mut snapshot);

        let folder = snapshot
            .hkcu_folder_types
            .types
            .get(TEST_FOLDER_GUID)
            .expect("folder type missing");

        assert_eq!(folder.canonical_name, None);
        assert!(snapshot
            .hkcu_folder_types
            .types
            .contains_key(TEST_FOLDER_GUID));

        let view = folder.top_views.get(TEST_VIEW_GUID).expect("view missing");
        assert_eq!(view.mode, Some(42));

        let errs: Vec<_> = snapshot
            .hkcu_folder_types
            .read_errors
            .iter()
            .filter(|e| e.operation == "value_size_limit")
            .collect();
        assert_eq!(errs.len(), 1);
        let err = &errs[0];
        assert_eq!(
            err.key_path,
            format!(r"{}\{}", TEST_FOLDER_TYPES, TEST_FOLDER_GUID)
        );
        assert!(err.error.contains("CanonicalName"));

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn folder_type_size_limit_produces_partial_status() {
        let reader = build_folder_types_fixture(None, true);

        let mut snapshot = InspectionSnapshot::default();
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        inspect_folder_types_hive(
            &reader,
            RootKey::Hklm,
            TEST_FOLDER_TYPES,
            ObservationSource::HklmFolderTypes,
            &mut snapshot.hklm_folder_types,
            4,
            &mut snapshot_budget,
        );

        finalize_inspection_status(&mut snapshot);

        let folder = snapshot
            .hklm_folder_types
            .types
            .get(TEST_FOLDER_GUID)
            .expect("folder type missing");

        let view = folder.top_views.get(TEST_VIEW_GUID).expect("view missing");

        assert!(!view.raw_values.contains_key("ColInfo"));
        assert_eq!(view.col_info, None);
        assert_eq!(view.mode, Some(42));

        assert!(snapshot_has_value_size_limit(&snapshot));

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));

        let errs: Vec<_> = snapshot
            .hklm_folder_types
            .read_errors
            .iter()
            .filter(|e| e.operation == "value_size_limit")
            .collect();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].root, RootKey::Hklm);
    }

    #[test]
    fn snapshot_budget_exact_capacity_allowed() {
        let mut budget = SnapshotBudget::new(8);

        assert!(budget.try_capture(4));
        assert!(budget.try_capture(4));

        assert_eq!(budget.captured_bytes, 8);
        assert!(!budget.try_capture(1));
        assert_eq!(budget.captured_bytes, 8);
    }

    #[test]
    fn snapshot_budget_rejected_capture_preserves_capacity() {
        let mut budget = SnapshotBudget::new(8);

        assert!(budget.try_capture(4));
        assert!(!budget.try_capture(5));
        assert_eq!(budget.captured_bytes, 4);

        assert!(budget.try_capture(4));
        assert_eq!(budget.captured_bytes, 8);

        assert!(!budget.try_capture(usize::MAX));
        assert_eq!(budget.captured_bytes, 8);
    }

    #[test]
    fn advanced_snapshot_budget_limits_combined_values() {
        use winview_core::registry::keys::advanced_values::{HIDE_FILE_EXT, USE_COMPACT_MODE};
        use winview_core::registry::keys::paths::ADVANCED;
        use winview_core::test_support::FakeRegistryReader;

        let reader = FakeRegistryReader::new();
        reader.insert_value(
            RootKey::Hkcu,
            ADVANCED,
            HIDE_FILE_EXT,
            4,
            42u32.to_le_bytes().to_vec(),
            Some(RegistryValue::Dword(42)),
        );
        reader.insert_value(
            RootKey::Hkcu,
            ADVANCED,
            USE_COMPACT_MODE,
            4,
            42u32.to_le_bytes().to_vec(),
            Some(RegistryValue::Dword(42)),
        );

        let mut snapshot = InspectionSnapshot::default();
        let mut snapshot_budget = SnapshotBudget::new(4);
        inspect_advanced(&reader, &mut snapshot, 4, &mut snapshot_budget);
        finalize_inspection_status(&mut snapshot);

        assert!(snapshot.advanced.values.contains_key(HIDE_FILE_EXT));
        assert!(!snapshot.advanced.values.contains_key(USE_COMPACT_MODE));
        assert_eq!(snapshot_budget.captured_bytes, 4);

        let snapshot_errs: Vec<_> = snapshot
            .advanced
            .read_errors
            .iter()
            .filter(|e| e.operation == "snapshot_size_limit")
            .collect();
        assert_eq!(snapshot_errs.len(), 1);
        assert_eq!(snapshot_errs[0].root, RootKey::Hkcu);
        assert_eq!(snapshot_errs[0].key_path, ADVANCED);
        assert!(snapshot_errs[0].error.contains(USE_COMPACT_MODE));

        let value_size_errs: Vec<_> = snapshot
            .advanced
            .read_errors
            .iter()
            .filter(|e| e.operation == "value_size_limit")
            .collect();
        assert_eq!(value_size_errs.len(), 0);

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn folder_type_snapshot_budget_exact_capacity() {
        let reader = build_folder_types_fixture(None, true);

        let mut snapshot = InspectionSnapshot::default();
        let mut snapshot_budget = SnapshotBudget::new(12);

        inspect_folder_types_hive(
            &reader,
            RootKey::Hkcu,
            TEST_FOLDER_TYPES,
            ObservationSource::HkcuFolderTypes,
            &mut snapshot.hkcu_folder_types,
            MAX_VALUE_BYTES,
            &mut snapshot_budget,
        );

        finalize_inspection_status(&mut snapshot);

        let folder = snapshot
            .hkcu_folder_types
            .types
            .get(TEST_FOLDER_GUID)
            .expect("folder type missing");

        let view = folder.top_views.get(TEST_VIEW_GUID).expect("view missing");

        assert_eq!(view.mode, Some(42));
        assert_eq!(view.col_info, Some(vec![1u8; 8]));
        assert_eq!(snapshot_budget.captured_bytes, 12);

        let errs: Vec<_> = snapshot
            .hkcu_folder_types
            .read_errors
            .iter()
            .filter(|e| e.operation == "snapshot_size_limit")
            .collect();
        assert_eq!(errs.len(), 0);

        assert!(matches!(snapshot.status, InspectionStatus::Complete));
    }

    #[test]
    fn folder_type_snapshot_budget_omits_excess_value() {
        let reader = build_folder_types_fixture(None, true);

        let mut snapshot = InspectionSnapshot::default();
        let mut snapshot_budget = SnapshotBudget::new(4);

        inspect_folder_types_hive(
            &reader,
            RootKey::Hkcu,
            TEST_FOLDER_TYPES,
            ObservationSource::HkcuFolderTypes,
            &mut snapshot.hkcu_folder_types,
            MAX_VALUE_BYTES,
            &mut snapshot_budget,
        );

        finalize_inspection_status(&mut snapshot);

        let folder = snapshot
            .hkcu_folder_types
            .types
            .get(TEST_FOLDER_GUID)
            .expect("folder type missing");

        let view = folder.top_views.get(TEST_VIEW_GUID).expect("view missing");

        assert_eq!(view.mode, Some(42));
        assert!(!view.raw_values.contains_key("ColInfo"));
        assert_eq!(view.col_info, None);
        assert_eq!(snapshot_budget.captured_bytes, 4);

        let errs: Vec<_> = snapshot
            .hkcu_folder_types
            .read_errors
            .iter()
            .filter(|e| e.operation == "snapshot_size_limit")
            .collect();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].root, RootKey::Hkcu);
        assert_eq!(
            errs[0].key_path,
            format!(
                r"{}\{}\TopViews\{}",
                TEST_FOLDER_TYPES, TEST_FOLDER_GUID, TEST_VIEW_GUID
            )
        );
        assert!(errs[0].error.contains("ColInfo"));

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn canonical_name_consumes_snapshot_budget() {
        let reader = build_folder_types_fixture(Some("A"), false);

        let mut snapshot = InspectionSnapshot::default();
        let mut snapshot_budget = SnapshotBudget::new(4);

        inspect_folder_types_hive(
            &reader,
            RootKey::Hkcu,
            TEST_FOLDER_TYPES,
            ObservationSource::HkcuFolderTypes,
            &mut snapshot.hkcu_folder_types,
            MAX_VALUE_BYTES,
            &mut snapshot_budget,
        );

        finalize_inspection_status(&mut snapshot);

        let folder = snapshot
            .hkcu_folder_types
            .types
            .get(TEST_FOLDER_GUID)
            .expect("folder type missing");

        assert_eq!(folder.canonical_name, Some("A".to_string()));

        let view = folder.top_views.get(TEST_VIEW_GUID).expect("view missing");

        assert!(!view.raw_values.contains_key("Mode"));
        assert_eq!(view.mode, None);
        assert_eq!(snapshot_budget.captured_bytes, 4);

        let errs: Vec<_> = snapshot
            .hkcu_folder_types
            .read_errors
            .iter()
            .filter(|e| e.operation == "snapshot_size_limit")
            .collect();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].root, RootKey::Hkcu);
        assert_eq!(
            errs[0].key_path,
            format!(
                r"{}\{}\TopViews\{}",
                TEST_FOLDER_TYPES, TEST_FOLDER_GUID, TEST_VIEW_GUID
            )
        );
        assert!(errs[0].error.contains("Mode"));

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn snapshot_budget_shared_across_folder_types_hives() {
        let reader = build_folder_types_fixture(None, false);

        let mut snapshot = InspectionSnapshot::default();
        let mut snapshot_budget = SnapshotBudget::new(4);

        inspect_folder_types_hive(
            &reader,
            RootKey::Hkcu,
            TEST_FOLDER_TYPES,
            ObservationSource::HkcuFolderTypes,
            &mut snapshot.hkcu_folder_types,
            MAX_VALUE_BYTES,
            &mut snapshot_budget,
        );

        inspect_folder_types_hive(
            &reader,
            RootKey::Hklm,
            TEST_FOLDER_TYPES,
            ObservationSource::HklmFolderTypes,
            &mut snapshot.hklm_folder_types,
            MAX_VALUE_BYTES,
            &mut snapshot_budget,
        );

        finalize_inspection_status(&mut snapshot);

        let hkcu_folder = snapshot
            .hkcu_folder_types
            .types
            .get(TEST_FOLDER_GUID)
            .expect("hkcu folder type missing");
        let hkcu_view = hkcu_folder
            .top_views
            .get(TEST_VIEW_GUID)
            .expect("hkcu view missing");
        assert_eq!(hkcu_view.mode, Some(42));

        let hklm_folder = snapshot
            .hklm_folder_types
            .types
            .get(TEST_FOLDER_GUID)
            .expect("hklm folder type missing");
        let hklm_view = hklm_folder
            .top_views
            .get(TEST_VIEW_GUID)
            .expect("hklm view missing");
        assert!(!hklm_view.raw_values.contains_key("Mode"));
        assert_eq!(hklm_view.mode, None);

        assert_eq!(snapshot_budget.captured_bytes, 4);

        let errs: Vec<_> = snapshot
            .hklm_folder_types
            .read_errors
            .iter()
            .filter(|e| e.operation == "snapshot_size_limit")
            .collect();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].root, RootKey::Hklm);

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }
}
