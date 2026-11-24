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

struct TraversalBudget {
    visited_keys: usize,
    max_keys: usize,
}

impl TraversalBudget {
    fn new(max_keys: usize) -> Self {
        Self {
            visited_keys: 0,
            max_keys,
        }
    }

    fn record_visit(&mut self) {
        self.visited_keys += 1;
    }

    #[allow(dead_code)]
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
        inspect_folder_types(self, &mut snapshot);
        inspect_bags(self, &mut snapshot);
        inspect_streams(self, &mut snapshot);
        inspect_advanced(self, &mut snapshot);
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
fn inspect_folder_types(reader: &WindowsRegistryReader, snapshot: &mut InspectionSnapshot) {
    use winview_core::registry::keys::paths::{HKCU_FOLDER_TYPES, HKLM_FOLDER_TYPES};
    inspect_folder_types_hive(
        reader,
        RootKey::Hkcu,
        HKCU_FOLDER_TYPES,
        ObservationSource::HkcuFolderTypes,
        &mut snapshot.hkcu_folder_types,
    );

    // HKLM FolderTypes
    inspect_folder_types_hive(
        reader,
        RootKey::Hklm,
        HKLM_FOLDER_TYPES,
        ObservationSource::HklmFolderTypes,
        &mut snapshot.hklm_folder_types,
    );
}

fn inspect_folder_types_hive(
    reader: &WindowsRegistryReader,
    root: RootKey,
    path: &str,
    source: ObservationSource,
    folder_types: &mut FolderTypesSnapshot,
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
        let canonical_name =
            guid_key
                .get_value(CANONICAL_NAME)
                .ok()
                .and_then(|(_, _, v, _)| match v {
                    RegistryValue::String(s) => Some(s),
                    _ => None,
                });

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

fn inspect_advanced(reader: &WindowsRegistryReader, snapshot: &mut InspectionSnapshot) {
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
        for (value_name, _type_name) in values {
            let value_result = key.get_value(&value_name);
            let read_result = match value_result {
                Ok((tn, raw_bytes, decoded, status)) => {
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
        }
    }

    target.insert(map_key.clone(), snapshot);
}
fn has_depth_limit_error(key: &KeySnapshot) -> bool {
    key.read_errors.iter().any(|e| e.operation == "depth_limit")
        || key.subkeys.values().any(has_depth_limit_error)
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
    use std::sync::Arc;
    use winview_core::model::{InspectionSnapshot, KeySnapshot, ObservationSource};

    struct FakeReader {
        data: Arc<BTreeMap<String, Vec<String>>>,
    }

    impl FakeReader {
        fn new(data: BTreeMap<String, Vec<String>>) -> Self {
            Self {
                data: Arc::new(data),
            }
        }
    }

    struct FakeKey {
        path: String,
        data: Arc<BTreeMap<String, Vec<String>>>,
    }

    impl RegistryKey for FakeKey {
        fn enum_values(&self) -> RegistryResult<Vec<(String, ValueTypeName)>> {
            Ok(vec![])
        }
        fn enum_subkeys(&self) -> RegistryResult<Vec<String>> {
            Ok(self.data.get(&self.path).cloned().unwrap_or_default())
        }
        fn get_value(
            &self,
            _name: &str,
        ) -> RegistryResult<(ValueTypeName, Vec<u8>, RegistryValue, DecodingStatus)> {
            Err(RegistryError::ValueNotFound {
                root: RootKey::Hkcu,
                path: "".into(),
                name: "".into(),
            })
        }
        fn get_value_type(&self, _name: &str) -> RegistryResult<ValueTypeName> {
            Ok(ValueTypeName::RegSz)
        }
        fn open_subkey(&self, name: &str) -> RegistryResult<Box<dyn RegistryKey>> {
            let subpath = format!("{}\\{}", self.path, name);
            if self.data.contains_key(&subpath) {
                Ok(Box::new(FakeKey {
                    path: subpath,
                    data: self.data.clone(),
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
        fn open_key(&self, _root: RootKey, path: &str) -> RegistryResult<Box<dyn RegistryKey>> {
            if self.data.contains_key(path) {
                Ok(Box::new(FakeKey {
                    path: path.to_string(),
                    data: self.data.clone(),
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
}
