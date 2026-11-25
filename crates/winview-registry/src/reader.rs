// winview-registry/src/reader.rs

use crate::version::detect_windows_version;
use std::collections::BTreeMap;
use std::ops::Deref;
use windows::core::PWSTR;
use windows::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{RegEnumKeyExW, RegEnumValueW, HKEY};
use windows_registry::{Key, Type as RegType, CURRENT_USER, LOCAL_MACHINE};
use winview_core::model::{
    DecodingStatus, FolderTypeSnapshot, FolderTypesSnapshot, InspectionError, InspectionSnapshot,
    InspectionStatus, KeySnapshot, ObservationSource, RawRegistryValue, ReadError, RegistryValue,
    ValueReadResult, ViewSnapshot,
};
use winview_core::registry::keys::{RootKey, ValueTypeName};
use winview_core::registry::traits::{RegistryError, RegistryKey, RegistryReader, RegistryResult};

const MAX_SUBKEY_NAME_UNITS: usize = 256;
const MAX_VALUE_NAME_UNITS: usize = 16_384;

fn registry_error_to_read_error(error: &RegistryError) -> ReadError {
    match error {
        RegistryError::KeyNotFound { path, .. } => ReadError::KeyNotFound { path: path.clone() },
        RegistryError::ValueNotFound { name, .. } => {
            ReadError::ValueNotFound { name: name.clone() }
        }
        RegistryError::AccessDenied { path, .. } => ReadError::AccessDenied { path: path.clone() },
        _ => ReadError::Other(error.to_string()),
    }
}

fn record_value_read_error(
    errors: &mut Vec<InspectionError>,
    root: RootKey,
    path: &str,
    value_name: &str,
    error: &RegistryError,
    optional: bool,
) {
    if optional && matches!(error, RegistryError::ValueNotFound { .. }) {
        return;
    }

    errors.push(InspectionError {
        key_path: path.to_string(),
        root,
        operation: "get_value".to_string(),
        error: format!("Registry value {:?}: {}", value_name, error),
    });
}

fn classify_windows_error(
    code: u32,
    root: RootKey,
    path: &str,
    value_name: Option<&str>,
    message: String,
) -> RegistryError {
    match code {
        0x80070002 => match value_name {
            Some(name) => RegistryError::ValueNotFound {
                root,
                path: path.to_string(),
                name: name.to_string(),
            },
            None => RegistryError::KeyNotFound {
                root,
                path: path.to_string(),
            },
        },
        0x80070003 => RegistryError::KeyNotFound {
            root,
            path: path.to_string(),
        },
        0x80070005 => RegistryError::AccessDenied {
            root,
            path: path.to_string(),
        },
        _ => RegistryError::WindowsApi(message),
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
            RootKey::Hkcu => CURRENT_USER.open(path).map_err(|e| {
                let code = e.code().0 as u32;
                classify_windows_error(code, root, path, None, e.to_string())
            })?,
            RootKey::Hklm => LOCAL_MACHINE.open(path).map_err(|e| {
                let code = e.code().0 as u32;
                classify_windows_error(code, root, path, None, e.to_string())
            })?,
        };
        Ok(Box::new(WindowsRegistryKey {
            key,
            root,
            path: path.to_string(),
        }))
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
        inspect_folder_types(
            self,
            &mut snapshot,
            &mut snapshot_budget,
            MAX_KEYS_PER_DOMAIN,
        );

        inspect_bags(self, &mut snapshot, &mut snapshot_budget);

        inspect_streams(self, &mut snapshot, &mut snapshot_budget);

        inspect_advanced(self, &mut snapshot, MAX_VALUE_BYTES, &mut snapshot_budget);

        inspect_clsid(self, &mut snapshot, &mut snapshot_budget);

        inspect_policies(self, &mut snapshot, &mut snapshot_budget);

        finalize_inspection_status(&mut snapshot);
        Ok(snapshot)
    }
}

fn decode_enumerated_name(
    status: WIN32_ERROR,
    buffer: &[u16],
    length: u32,
    root: RootKey,
    path: &str,
    operation: &str,
    index: u32,
) -> RegistryResult<Option<String>> {
    if status == ERROR_NO_MORE_ITEMS {
        return Ok(None);
    }

    if status != ERROR_SUCCESS {
        return Err(RegistryError::WindowsApi(format!(
            "{} failed at {:?}\\{} index {}: \
                 Win32 error {}",
            operation, root, path, index, status.0,
        )));
    }

    let length = length as usize;

    if length >= buffer.len() {
        return Err(RegistryError::InvalidData(format!(
            "{} returned invalid name length {} \
                 at {:?}\\{} index {}",
            operation, length, root, path, index,
        )));
    }

    let units = &buffer[..length];

    if units.contains(&0) {
        return Err(RegistryError::InvalidData(format!(
            "{} returned an embedded null \
                 at {:?}\\{} index {}",
            operation, root, path, index,
        )));
    }

    let name = String::from_utf16(units).map_err(|_| {
        RegistryError::InvalidData(format!(
            "{} returned invalid UTF-16 \
                     at {:?}\\{} index {}",
            operation, root, path, index,
        ))
    })?;

    Ok(Some(name))
}

fn collect_registry_entries<T>(
    mut read_entry: impl FnMut(u32) -> RegistryResult<Option<T>>,
) -> RegistryResult<Vec<T>> {
    let mut entries = Vec::new();

    for index in 0..=u32::MAX {
        match read_entry(index)? {
            Some(entry) => entries.push(entry),
            None => return Ok(entries),
        }
    }

    Err(RegistryError::InvalidData(
        "Registry enumeration index exhausted".into(),
    ))
}

struct WindowsRegistryKey {
    key: Key,
    root: RootKey,
    path: String,
}

impl RegistryKey for WindowsRegistryKey {
    fn enum_values(&self) -> RegistryResult<Vec<(String, ValueTypeName)>> {
        let handle = HKEY(self.key.as_raw());

        let mut buffer = vec![0u16; MAX_VALUE_NAME_UNITS];

        collect_registry_entries(|index| {
            let mut length = buffer.len() as u32;
            let mut raw_type = 0u32;

            // SAFETY:
            // The borrowed registry handle remains
            // valid for this synchronous call.
            // All output pointers reference valid,
            // writable storage.
            // Value payload retrieval is disabled.
            let status = unsafe {
                RegEnumValueW(
                    handle,
                    index,
                    Some(PWSTR(buffer.as_mut_ptr())),
                    &mut length,
                    None,
                    Some(&mut raw_type),
                    None,
                    None,
                )
            };

            let name = decode_enumerated_name(
                status,
                &buffer,
                length,
                self.root,
                &self.path,
                "RegEnumValueW",
                index,
            )?;

            Ok(name.map(|name| (name, convert_type(RegType::from(raw_type)))))
        })
    }

    fn enum_subkeys(&self) -> RegistryResult<Vec<String>> {
        let handle = HKEY(self.key.as_raw());

        let mut buffer = vec![0u16; MAX_SUBKEY_NAME_UNITS];

        collect_registry_entries(|index| {
            let mut length = buffer.len() as u32;

            // SAFETY:
            // The borrowed registry handle remains
            // valid for this synchronous call.
            // The UTF-16 buffer and length pointer
            // are writable and correctly sized.
            // Unused optional pointers are null.
            let status = unsafe {
                RegEnumKeyExW(
                    handle,
                    index,
                    Some(PWSTR(buffer.as_mut_ptr())),
                    &mut length,
                    None,
                    None,
                    None,
                    None,
                )
            };

            decode_enumerated_name(
                status,
                &buffer,
                length,
                self.root,
                &self.path,
                "RegEnumKeyExW",
                index,
            )
        })
    }

    fn get_value(
        &self,
        name: &str,
    ) -> RegistryResult<(ValueTypeName, Vec<u8>, RegistryValue, DecodingStatus)> {
        let value = self.key.get_value(name).map_err(|e| {
            let code = e.code().0 as u32;
            classify_windows_error(code, self.root, &self.path, Some(name), e.to_string())
        })?;

        let (type_name, reg_value, raw_bytes, status) =
            convert_registry_value(value.ty(), value.deref());
        Ok((type_name, raw_bytes, reg_value, status))
    }

    fn get_value_type(&self, name: &str) -> RegistryResult<ValueTypeName> {
        let vtype = self.key.get_type(name).map_err(|e| {
            let code = e.code().0 as u32;
            classify_windows_error(code, self.root, &self.path, Some(name), e.to_string())
        })?;

        Ok(convert_type(vtype))
    }

    fn open_subkey(&self, name: &str) -> RegistryResult<Box<dyn RegistryKey>> {
        let child_path = format!(r"{}\{}", self.path, name);
        let subkey = self.key.open(name).map_err(|e| {
            let code = e.code().0 as u32;
            classify_windows_error(code, self.root, &child_path, None, e.to_string())
        })?;
        Ok(Box::new(WindowsRegistryKey {
            key: subkey,
            root: self.root,
            path: child_path,
        }))
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

/// Decodes a UTF-16LE REG_MULTI_SZ sequence.
/// An empty string terminates the sequence.
/// Original registry bytes are preserved by the caller.
fn decode_reg_multi_sz(data: &[u8]) -> Result<Vec<String>, String> {
    if !data.len().is_multiple_of(2) {
        return Err("Odd byte length for REG_MULTI_SZ".into());
    }

    let units: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();

    // Preserve existing compatibility with empty payloads.
    if units.is_empty() {
        return Ok(Vec::new());
    }

    let mut result = Vec::new();
    let mut start = 0;

    for (index, &unit) in units.iter().enumerate() {
        if unit != 0 {
            continue;
        }

        // An empty element terminates the sequence.
        if index == start {
            return Ok(result);
        }

        let element = String::from_utf16(&units[start..index])
            .map_err(|_| "Invalid UTF-16 in multi-string element".to_string())?;

        result.push(element);
        start = index + 1;
    }

    Err("Missing REG_MULTI_SZ sequence terminator".into())
}

// Inspection modules - collect errors instead of failing
fn inspect_folder_types(
    reader: &dyn RegistryReader,
    snapshot: &mut InspectionSnapshot,
    snapshot_budget: &mut SnapshotBudget,
    max_keys: usize,
) {
    use winview_core::registry::keys::paths::{HKCU_FOLDER_TYPES, HKLM_FOLDER_TYPES};

    let mut traversal_budget = TraversalBudget::new(max_keys);

    inspect_folder_types_hive(
        reader,
        RootKey::Hkcu,
        HKCU_FOLDER_TYPES,
        ObservationSource::HkcuFolderTypes,
        &mut snapshot.hkcu_folder_types,
        MAX_VALUE_BYTES,
        snapshot_budget,
        &mut traversal_budget,
    );

    inspect_folder_types_hive(
        reader,
        RootKey::Hklm,
        HKLM_FOLDER_TYPES,
        ObservationSource::HklmFolderTypes,
        &mut snapshot.hklm_folder_types,
        MAX_VALUE_BYTES,
        snapshot_budget,
        &mut traversal_budget,
    );
}

fn reserve_folder_type_key_open(
    budget: &mut TraversalBudget,
    folder_types: &mut FolderTypesSnapshot,
    root: RootKey,
    path: &str,
) -> bool {
    if budget.limit_exceeded {
        return false;
    }

    if budget.is_exhausted() {
        budget.limit_exceeded = true;

        folder_types.read_errors.push(InspectionError {
            key_path: path.to_string(),
            root,
            operation: "key_limit".to_string(),
            error: format!(
                "Maximum registry keys per domain ({}) exceeded",
                budget.max_keys
            ),
        });

        return false;
    }

    budget.record_visit();

    true
}

#[allow(clippy::too_many_arguments)]
fn inspect_folder_types_hive(
    reader: &dyn RegistryReader,
    root: RootKey,
    path: &str,
    source: ObservationSource,
    folder_types: &mut FolderTypesSnapshot,
    max_value_bytes: usize,
    snapshot_budget: &mut SnapshotBudget,
    traversal_budget: &mut TraversalBudget,
) {
    use winview_core::registry::keys::paths::{
        CANONICAL_NAME, COLUMN_LIST, COL_INFO, F_FLAGS, GROUP_ASCENDING, GROUP_BY, ICON_SIZE,
        LOGICAL_VIEW_MODE, MODE, SORT_BY_LIST,
    };

    if !reserve_folder_type_key_open(traversal_budget, folder_types, root, path) {
        return;
    }

    // Open the FolderTypes key
    let key_result = reader.open_key(root, path);
    let key = match key_result {
        Ok(k) => k,
        Err(RegistryError::KeyNotFound { .. }) if matches!(root, RootKey::Hkcu) => {
            return;
        }
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
        if !reserve_folder_type_key_open(traversal_budget, folder_types, root, &guid_path) {
            break;
        }
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
            Err(e) => {
                record_value_read_error(
                    &mut folder_types.read_errors,
                    root,
                    &guid_path,
                    CANONICAL_NAME,
                    &e,
                    true,
                );

                None
            }
        };

        // Open TopViews\{00000000-0000-0000-0000-000000000000}
        let top_views_path = format!(
            r"{}\TopViews\{{00000000-0000-0000-0000-000000000000}}",
            guid_path
        );
        if !reserve_folder_type_key_open(traversal_budget, folder_types, root, &top_views_path) {
            folder_types.types.insert(
                guid.clone(),
                FolderTypeSnapshot {
                    guid,
                    canonical_name,
                    top_views: BTreeMap::new(),
                    read_errors: Vec::new(),
                },
            );

            break;
        }
        let top_views_key_result = reader.open_key(root, &top_views_path);
        let top_views_key = match top_views_key_result {
            Ok(k) => k,
            Err(e) => {
                if !matches!(e, RegistryError::KeyNotFound { .. }) {
                    folder_types.read_errors.push(InspectionError {
                        key_path: top_views_path,
                        root,
                        operation: "open_key".to_string(),
                        error: e.to_string(),
                    });
                }

                let folder_type_snapshot = FolderTypeSnapshot {
                    guid: guid.clone(),
                    canonical_name,
                    top_views: BTreeMap::new(),
                    read_errors: Vec::new(),
                };

                folder_types.types.insert(guid, folder_type_snapshot);

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
                Err(e) => {
                    record_value_read_error(
                        &mut folder_types.read_errors,
                        root,
                        &top_views_path,
                        const_name,
                        &e,
                        true,
                    );

                    ValueReadResult::Failure(registry_error_to_read_error(&e))
                }
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

fn inspect_bags(
    reader: &dyn RegistryReader,
    snapshot: &mut InspectionSnapshot,
    snapshot_budget: &mut SnapshotBudget,
) {
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
        KeyPresence::Optional,
        0,
        &mut budget,
        snapshot_budget,
    );
    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        BAGS,
        ObservationSource::HkcuBags,
        &mut snapshot.bags,
        "",
        KeyPresence::Optional,
        0,
        &mut budget,
        snapshot_budget,
    );
    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        ALL_FOLDERS_SHELL,
        ObservationSource::HkcuBags,
        &mut snapshot.bags,
        "",
        KeyPresence::Optional,
        0,
        &mut budget,
        snapshot_budget,
    );
    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        ALL_FOLDERS_COMDLG,
        ObservationSource::HkcuBags,
        &mut snapshot.bags,
        "",
        KeyPresence::Optional,
        0,
        &mut budget,
        snapshot_budget,
    );
    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        ALL_FOLDERS_COMDLG_LEGACY,
        ObservationSource::HkcuBags,
        &mut snapshot.bags,
        "",
        KeyPresence::Optional,
        0,
        &mut budget,
        snapshot_budget,
    );
}

fn inspect_streams(
    reader: &dyn RegistryReader,
    snapshot: &mut InspectionSnapshot,
    snapshot_budget: &mut SnapshotBudget,
) {
    use winview_core::registry::keys::paths::{DESKTOP, STREAMS, STREAMS_DEFAULTS};

    let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);

    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        STREAMS,
        ObservationSource::HkcuStreams,
        &mut snapshot.streams,
        "",
        KeyPresence::Optional,
        0,
        &mut budget,
        snapshot_budget,
    );
    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        STREAMS_DEFAULTS,
        ObservationSource::HkcuStreams,
        &mut snapshot.streams,
        "",
        KeyPresence::Optional,
        0,
        &mut budget,
        snapshot_budget,
    );
    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        DESKTOP,
        ObservationSource::HkcuStreams,
        &mut snapshot.streams,
        "",
        KeyPresence::Optional,
        0,
        &mut budget,
        snapshot_budget,
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
        Err(RegistryError::KeyNotFound { .. }) => {
            return;
        }
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
            Err(e) => {
                record_value_read_error(
                    &mut snapshot.advanced.read_errors,
                    RootKey::Hkcu,
                    ADVANCED,
                    value_name,
                    &e,
                    true,
                );

                ValueReadResult::Failure(registry_error_to_read_error(&e))
            }
        };
        snapshot
            .advanced
            .values
            .insert(value_name.to_string(), read_result);
    }
}

fn inspect_clsid(
    reader: &dyn RegistryReader,
    snapshot: &mut InspectionSnapshot,
    snapshot_budget: &mut SnapshotBudget,
) {
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
            KeyPresence::Optional,
            0,
            &mut budget,
            snapshot_budget,
        );
    }
}

fn inspect_policies(
    reader: &dyn RegistryReader,
    snapshot: &mut InspectionSnapshot,
    snapshot_budget: &mut SnapshotBudget,
) {
    use winview_core::registry::keys::paths::*;

    let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);

    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        EXPLORER_POLICIES,
        ObservationSource::HkcuPolicies,
        &mut snapshot.policies,
        "",
        KeyPresence::Optional,
        0,
        &mut budget,
        snapshot_budget,
    );
    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        USER_EXPLORER_POLICIES,
        ObservationSource::HkcuPolicies,
        &mut snapshot.policies,
        "",
        KeyPresence::Optional,
        0,
        &mut budget,
        snapshot_budget,
    );
    inspect_key_tree(
        reader,
        RootKey::Hklm,
        EXPLORER_POLICIES,
        ObservationSource::HklmPolicies,
        &mut snapshot.policies,
        "HKLM",
        KeyPresence::Optional,
        0,
        &mut budget,
        snapshot_budget,
    );
    inspect_key_tree(
        reader,
        RootKey::Hkcu,
        USER_COMDLG_PLACESBAR,
        ObservationSource::HkcuPolicies,
        &mut snapshot.policies,
        "",
        KeyPresence::Optional,
        0,
        &mut budget,
        snapshot_budget,
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

#[derive(Clone, Copy)]
enum KeyPresence {
    Optional,
    Required,
}

#[allow(clippy::too_many_arguments)]
fn inspect_key_tree(
    reader: &dyn RegistryReader,
    root: RootKey,
    path: &str,
    source: ObservationSource,
    target: &mut BTreeMap<String, KeySnapshot>,
    key_prefix: &str,
    presence: KeyPresence,
    depth: usize,
    budget: &mut TraversalBudget,
    snapshot_budget: &mut SnapshotBudget,
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

        Err(RegistryError::KeyNotFound { .. }) if matches!(presence, KeyPresence::Optional) => {
            return;
        }

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
    match key.enum_values() {
        Ok(values) => {
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

                        if let Some(error) = check_snapshot_budget(
                            snapshot_budget,
                            root,
                            path,
                            &value_name,
                            raw_bytes.len(),
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
                    Err(e) => {
                        record_value_read_error(
                            &mut snapshot.read_errors,
                            root,
                            path,
                            &value_name,
                            &e,
                            false,
                        );

                        ValueReadResult::Failure(registry_error_to_read_error(&e))
                    }
                };
                snapshot.values.insert(value_name, read_result);
            }
        }
        Err(e) => {
            snapshot.read_errors.push(InspectionError {
                key_path: path.to_string(),
                root,
                operation: "enum_values".to_string(),
                error: e.to_string(),
            });
        }
    }

    // Recursively read subkeys (with depth limit)
    match key.enum_subkeys() {
        Ok(subkeys) => {
            for subkey_name in subkeys {
                let subkey_path = format!(r"{}\{}", path, subkey_name);
                inspect_key_tree(
                    reader,
                    root,
                    &subkey_path,
                    source,
                    &mut snapshot.subkeys,
                    "",
                    KeyPresence::Required,
                    depth + 1,
                    budget,
                    snapshot_budget,
                );
                if budget.limit_exceeded {
                    break;
                }
            }
        }
        Err(e) => {
            snapshot.read_errors.push(InspectionError {
                key_path: path.to_string(),
                root,
                operation: "enum_subkeys".to_string(),
                error: e.to_string(),
            });
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
    snapshot
        .hkcu_folder_types
        .read_errors
        .iter()
        .any(|e| e.operation == "key_limit")
        || snapshot
            .hklm_folder_types
            .read_errors
            .iter()
            .any(|e| e.operation == "key_limit")
        || [
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

fn has_snapshot_size_limit_error(key: &KeySnapshot) -> bool {
    key.read_errors
        .iter()
        .any(|e| e.operation == "snapshot_size_limit")
        || key.subkeys.values().any(has_snapshot_size_limit_error)
}

fn snapshot_has_snapshot_size_limit(snapshot: &InspectionSnapshot) -> bool {
    has_snapshot_size_limit_error(&snapshot.advanced)
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
        || [
            &snapshot.bags,
            &snapshot.streams,
            &snapshot.clsid,
            &snapshot.policies,
        ]
        .iter()
        .any(|map| map.values().any(has_snapshot_size_limit_error))
}

fn count_key_read_errors(key: &KeySnapshot) -> usize {
    key.read_errors.len()
        + key
            .subkeys
            .values()
            .map(count_key_read_errors)
            .sum::<usize>()
}

fn count_folder_types_read_errors(folder_types: &FolderTypesSnapshot) -> usize {
    folder_types.read_errors.len()
        + folder_types
            .types
            .values()
            .map(|folder| folder.read_errors.len())
            .sum::<usize>()
}

fn count_snapshot_read_errors(snapshot: &InspectionSnapshot) -> usize {
    snapshot.errors.len()
        + count_folder_types_read_errors(&snapshot.hkcu_folder_types)
        + count_folder_types_read_errors(&snapshot.hklm_folder_types)
        + count_key_read_errors(&snapshot.advanced)
        + [
            &snapshot.bags,
            &snapshot.streams,
            &snapshot.clsid,
            &snapshot.policies,
        ]
        .iter()
        .map(|domain| domain.values().map(count_key_read_errors).sum::<usize>())
        .sum::<usize>()
}

fn key_has_successful_observations(key: &KeySnapshot) -> bool {
    key.values
        .values()
        .any(|value| matches!(value, ValueReadResult::Success(_)))
        || key.subkeys.values().any(key_has_successful_observations)
}

fn folder_types_has_successful_observations(folder_types: &FolderTypesSnapshot) -> bool {
    folder_types.types.values().any(|folder| {
        folder.canonical_name.is_some()
            || folder.top_views.values().any(|view| {
                view.raw_values
                    .values()
                    .any(|value| matches!(value, ValueReadResult::Success(_)))
            })
    })
}

fn snapshot_has_successful_observations(snapshot: &InspectionSnapshot) -> bool {
    folder_types_has_successful_observations(&snapshot.hkcu_folder_types)
        || folder_types_has_successful_observations(&snapshot.hklm_folder_types)
        || key_has_successful_observations(&snapshot.advanced)
        || [
            &snapshot.bags,
            &snapshot.streams,
            &snapshot.clsid,
            &snapshot.policies,
        ]
        .iter()
        .any(|domain| domain.values().any(key_has_successful_observations))
}

fn finalize_inspection_status(snapshot: &mut InspectionSnapshot) {
    let total = count_snapshot_read_errors(snapshot);

    let has_data = snapshot_has_successful_observations(snapshot);

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
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};
    use winview_core::model::{InspectionSnapshot, KeySnapshot, ObservationSource};

    type FakeValue = (u32, Vec<u8>, Option<RegistryValue>, DecodingStatus);

    type FakeValues = BTreeMap<String, BTreeMap<String, FakeValue>>;

    #[derive(Clone, Copy)]
    enum InjectedValueFailure {
        AccessDenied,
        ValueNotFound,
    }

    type ValueFailures = BTreeMap<(String, String), InjectedValueFailure>;

    #[derive(Default)]
    struct EnumFailures {
        values: BTreeSet<String>,
        subkeys: BTreeSet<String>,
    }

    struct FakeReader {
        data: Arc<BTreeMap<String, Vec<String>>>,
        opened_keys: Mutex<Vec<(RootKey, String)>>,
        values: Arc<FakeValues>,
        enum_failures: Arc<EnumFailures>,
        value_failures: Arc<ValueFailures>,
        denied_keys: BTreeSet<String>,
    }

    impl FakeReader {
        fn new(data: BTreeMap<String, Vec<String>>) -> Self {
            Self {
                data: Arc::new(data),
                opened_keys: Mutex::new(Vec::new()),
                values: Arc::new(FakeValues::new()),
                enum_failures: Arc::new(EnumFailures::default()),
                value_failures: Arc::new(BTreeMap::new()),
                denied_keys: BTreeSet::new(),
            }
        }

        fn with_values(data: BTreeMap<String, Vec<String>>, values: FakeValues) -> Self {
            Self {
                data: Arc::new(data),
                opened_keys: Mutex::new(Vec::new()),
                values: Arc::new(values),
                enum_failures: Arc::new(EnumFailures::default()),
                value_failures: Arc::new(BTreeMap::new()),
                denied_keys: BTreeSet::new(),
            }
        }

        fn with_denied_key(mut self, path: &str) -> Self {
            self.denied_keys.insert(path.to_string());

            self
        }

        fn with_value_failure(
            mut self,
            path: &str,
            name: &str,
            failure: InjectedValueFailure,
        ) -> Self {
            Arc::make_mut(&mut self.value_failures)
                .insert((path.to_string(), name.to_string()), failure);

            self
        }

        fn with_enum_failures(mut self, value_paths: &[&str], subkey_paths: &[&str]) -> Self {
            self.enum_failures = Arc::new(EnumFailures {
                values: value_paths.iter().map(|path| path.to_string()).collect(),
                subkeys: subkey_paths.iter().map(|path| path.to_string()).collect(),
            });

            self
        }
    }

    struct FakeKey {
        path: String,
        data: Arc<BTreeMap<String, Vec<String>>>,
        values: Arc<FakeValues>,
        enum_failures: Arc<EnumFailures>,
        value_failures: Arc<ValueFailures>,
    }

    impl RegistryKey for FakeKey {
        fn enum_values(&self) -> RegistryResult<Vec<(String, ValueTypeName)>> {
            if self.enum_failures.values.contains(&self.path) {
                return Err(RegistryError::WindowsApi(
                    "injected value enumeration failure".into(),
                ));
            }
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
            if self.enum_failures.subkeys.contains(&self.path) {
                return Err(RegistryError::WindowsApi(
                    "injected subkey enumeration failure".into(),
                ));
            }
            Ok(self.data.get(&self.path).cloned().unwrap_or_default())
        }
        fn get_value(
            &self,
            name: &str,
        ) -> RegistryResult<(ValueTypeName, Vec<u8>, RegistryValue, DecodingStatus)> {
            if let Some(failure) = self
                .value_failures
                .get(&(self.path.clone(), name.to_string()))
            {
                return Err(match failure {
                    InjectedValueFailure::AccessDenied => RegistryError::AccessDenied {
                        root: RootKey::Hkcu,
                        path: self.path.clone(),
                    },
                    InjectedValueFailure::ValueNotFound => RegistryError::ValueNotFound {
                        root: RootKey::Hkcu,
                        path: self.path.clone(),
                        name: name.to_string(),
                    },
                });
            }

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
                    path: self.path.clone(),
                    name: name.to_string(),
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
                    enum_failures: self.enum_failures.clone(),
                    value_failures: self.value_failures.clone(),
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

            if self.denied_keys.contains(path) {
                return Err(RegistryError::AccessDenied {
                    root,
                    path: path.to_string(),
                });
            }

            if self.data.contains_key(path) {
                Ok(Box::new(FakeKey {
                    path: path.to_string(),
                    data: self.data.clone(),
                    values: self.values.clone(),
                    enum_failures: self.enum_failures.clone(),
                    value_failures: self.value_failures.clone(),
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
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        inspect_key_tree(
            reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
        );

        finalize_inspection_status(&mut snapshot);

        (snapshot, budget)
    }

    fn inspect_optional_test_tree(
        reader: &dyn RegistryReader,
        max_keys: usize,
    ) -> (InspectionSnapshot, TraversalBudget) {
        let mut snapshot = InspectionSnapshot::default();

        let mut budget = TraversalBudget::new(max_keys);

        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        inspect_key_tree(
            reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            KeyPresence::Optional,
            0,
            &mut budget,
            &mut snapshot_budget,
        );

        finalize_inspection_status(&mut snapshot);

        (snapshot, budget)
    }

    fn inspect_specialized_folder_types(
        reader: &dyn RegistryReader,
        root: RootKey,
    ) -> (InspectionSnapshot, SnapshotBudget) {
        let mut snapshot = InspectionSnapshot::default();

        let mut budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        let (source, target) = match root {
            RootKey::Hkcu => (
                ObservationSource::HkcuFolderTypes,
                &mut snapshot.hkcu_folder_types,
            ),
            RootKey::Hklm => (
                ObservationSource::HklmFolderTypes,
                &mut snapshot.hklm_folder_types,
            ),
        };

        inspect_folder_types_hive(
            reader,
            root,
            TEST_FOLDER_TYPES,
            source,
            target,
            MAX_VALUE_BYTES,
            &mut budget,
            &mut TraversalBudget::new(MAX_KEYS_PER_DOMAIN),
        );

        finalize_inspection_status(&mut snapshot);

        (snapshot, budget)
    }

    fn inspect_folder_types_with_key_limit(
        reader: &dyn RegistryReader,
        max_keys: usize,
    ) -> (InspectionSnapshot, SnapshotBudget, TraversalBudget) {
        let mut snapshot = InspectionSnapshot::default();

        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        let mut traversal_budget = TraversalBudget::new(max_keys);

        inspect_folder_types_hive(
            reader,
            RootKey::Hkcu,
            TEST_FOLDER_TYPES,
            ObservationSource::HkcuFolderTypes,
            &mut snapshot.hkcu_folder_types,
            MAX_VALUE_BYTES,
            &mut snapshot_budget,
            &mut traversal_budget,
        );

        finalize_inspection_status(&mut snapshot);

        (snapshot, snapshot_budget, traversal_budget)
    }

    fn assert_folder_types_key_limit(snapshot: &InspectionSnapshot) {
        assert!(snapshot_has_key_limit(snapshot));

        assert!(matches!(
            &snapshot.status,
            InspectionStatus::Partial { reason }
                if reason ==
                    "Maximum registry key count exceeded"
        ));

        let limit_errors = [&snapshot.hkcu_folder_types, &snapshot.hklm_folder_types]
            .iter()
            .flat_map(|folder_types| folder_types.read_errors.iter())
            .filter(|error| error.operation == "key_limit")
            .count();

        assert_eq!(limit_errors, 1);
    }

    #[test]
    fn depth_below_limit_no_error() {
        let reader = build_hierarchy(30);
        let mut target = BTreeMap::new();
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut target,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
        );
        assert!(target.values().all(|ks| !has_depth_limit(ks)));
    }

    #[test]
    fn depth_at_boundary_no_error() {
        let reader = build_hierarchy(31);
        let mut target = BTreeMap::new();
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut target,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
        );
        assert!(target.values().all(|ks| !has_depth_limit(ks)));
    }

    #[test]
    fn depth_beyond_limit_blocked() {
        let reader = build_hierarchy(32);
        let mut target = BTreeMap::new();
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut target,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
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
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut target,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
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
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut target,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
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
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root1",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
        );
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root2",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
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
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
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
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
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
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
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
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
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
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
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
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
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
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
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
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
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
            &mut TraversalBudget::new(MAX_KEYS_PER_DOMAIN),
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
            &mut TraversalBudget::new(MAX_KEYS_PER_DOMAIN),
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
            &mut TraversalBudget::new(MAX_KEYS_PER_DOMAIN),
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
            &mut TraversalBudget::new(MAX_KEYS_PER_DOMAIN),
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
            &mut TraversalBudget::new(MAX_KEYS_PER_DOMAIN),
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
            &mut TraversalBudget::new(MAX_KEYS_PER_DOMAIN),
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
            &mut TraversalBudget::new(MAX_KEYS_PER_DOMAIN),
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
            &mut TraversalBudget::new(MAX_KEYS_PER_DOMAIN),
        );

        inspect_folder_types_hive(
            &reader,
            RootKey::Hklm,
            TEST_FOLDER_TYPES,
            ObservationSource::HklmFolderTypes,
            &mut snapshot.hklm_folder_types,
            MAX_VALUE_BYTES,
            &mut snapshot_budget,
            &mut TraversalBudget::new(MAX_KEYS_PER_DOMAIN),
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

    #[test]
    fn recursive_snapshot_budget_exact_capacity() {
        let reader = build_hierarchy(1);
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        let mut snapshot_budget = SnapshotBudget::new(8);

        let mut snapshot = InspectionSnapshot::default();
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
        );
        finalize_inspection_status(&mut snapshot);

        let root = snapshot.bags.get("root").expect("root missing");
        let child = root.subkeys.get(r"root\child").expect("child missing");

        for ks in [root, child] {
            let value = ks.values.get("TestValue").expect("TestValue missing");
            match value {
                ValueReadResult::Success(r) => {
                    assert_eq!(r.raw_bytes, vec![42u8, 0, 0, 0]);
                    assert!(matches!(r.decoded.as_ref(), Some(RegistryValue::Dword(42))));
                }
                _ => panic!("expected successful value read"),
            }
        }

        assert_eq!(snapshot_budget.captured_bytes, 8);

        let errs: Vec<_> = snapshot
            .bags
            .values()
            .flat_map(|ks| {
                ks.read_errors
                    .iter()
                    .filter(|e| e.operation == "snapshot_size_limit")
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(errs.len(), 0);

        assert!(matches!(snapshot.status, InspectionStatus::Complete));
    }

    #[test]
    fn recursive_snapshot_budget_omits_excess_value() {
        let reader = build_hierarchy(2);
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        let mut snapshot_budget = SnapshotBudget::new(8);

        let mut snapshot = InspectionSnapshot::default();
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
        );
        finalize_inspection_status(&mut snapshot);

        let root = snapshot.bags.get("root").expect("root missing");
        let child = root.subkeys.get(r"root\child").expect("child missing");
        let grandchild = root
            .subkeys
            .get(r"root\child")
            .and_then(|k| k.subkeys.get(r"root\child\child"))
            .expect("grandchild missing");

        for ks in [root, child] {
            let value = ks.values.get("TestValue").expect("TestValue missing");
            match value {
                ValueReadResult::Success(r) => {
                    assert_eq!(r.raw_bytes, vec![42u8, 0, 0, 0]);
                    assert!(matches!(r.decoded.as_ref(), Some(RegistryValue::Dword(42))));
                }
                _ => panic!("expected successful value read"),
            }
        }

        assert!(!grandchild.values.contains_key("TestValue"));

        let errs: Vec<_> = grandchild
            .read_errors
            .iter()
            .filter(|e| e.operation == "snapshot_size_limit")
            .collect();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].root, RootKey::Hkcu);
        assert_eq!(errs[0].key_path, r"root\child\child");
        assert!(errs[0].error.contains("TestValue"));

        assert_eq!(snapshot_budget.captured_bytes, 8);
        assert!(!budget.limit_exceeded);

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn recursive_snapshot_budget_rejection_allows_smaller_value() {
        let mut data = BTreeMap::new();
        data.insert("root".to_string(), Vec::new());

        let mut values = BTreeMap::new();
        let mut root_values = BTreeMap::new();
        root_values.insert(
            "A_First".to_string(),
            (
                4,
                42u32.to_le_bytes().to_vec(),
                Some(RegistryValue::Dword(42)),
                DecodingStatus::Decoded,
            ),
        );
        root_values.insert(
            "B_Large".to_string(),
            (
                11,
                1u64.to_le_bytes().to_vec(),
                Some(RegistryValue::Qword(1)),
                DecodingStatus::Decoded,
            ),
        );
        root_values.insert(
            "C_Last".to_string(),
            (
                4,
                42u32.to_le_bytes().to_vec(),
                Some(RegistryValue::Dword(42)),
                DecodingStatus::Decoded,
            ),
        );
        values.insert("root".to_string(), root_values);

        let reader = FakeReader::with_values(data, values);
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        let mut snapshot_budget = SnapshotBudget::new(8);

        let mut snapshot = InspectionSnapshot::default();
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
        );
        finalize_inspection_status(&mut snapshot);

        let root = snapshot.bags.get("root").expect("root missing");

        let first = root.values.get("A_First").expect("A_First missing");
        match first {
            ValueReadResult::Success(r) => {
                assert_eq!(r.raw_bytes, vec![42u8, 0, 0, 0]);
                assert!(matches!(r.decoded.as_ref(), Some(RegistryValue::Dword(42))));
            }
            _ => panic!("expected successful A_First read"),
        }

        assert!(!root.values.contains_key("B_Large"));

        let last = root.values.get("C_Last").expect("C_Last missing");
        match last {
            ValueReadResult::Success(r) => {
                assert_eq!(r.raw_bytes, vec![42u8, 0, 0, 0]);
                assert!(matches!(r.decoded.as_ref(), Some(RegistryValue::Dword(42))));
            }
            _ => panic!("expected successful C_Last read"),
        }

        assert_eq!(snapshot_budget.captured_bytes, 8);

        let errs: Vec<_> = root
            .read_errors
            .iter()
            .filter(|e| e.operation == "snapshot_size_limit")
            .collect();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].root, RootKey::Hkcu);
        assert_eq!(errs[0].key_path, "root");
        assert!(errs[0].error.contains("B_Large"));
    }

    #[test]
    fn snapshot_budget_shared_across_recursive_domains() {
        let reader = build_hierarchy(0);
        let mut snapshot = InspectionSnapshot::default();
        let mut snapshot_budget = SnapshotBudget::new(4);

        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
        );

        let mut budget2 = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuStreams,
            &mut snapshot.streams,
            "",
            KeyPresence::Required,
            0,
            &mut budget2,
            &mut snapshot_budget,
        );

        finalize_inspection_status(&mut snapshot);

        let bags_root = snapshot.bags.get("root").expect("bags root missing");
        assert!(bags_root.values.contains_key("TestValue"));

        let streams_root = snapshot.streams.get("root").expect("streams root missing");
        assert!(!streams_root.values.contains_key("TestValue"));

        let errs: Vec<_> = streams_root
            .read_errors
            .iter()
            .filter(|e| e.operation == "snapshot_size_limit")
            .collect();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].root, RootKey::Hkcu);

        assert_eq!(snapshot_budget.captured_bytes, 4);

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn snapshot_budget_shared_across_specialized_and_recursive_domains() {
        let folder_reader = build_folder_types_fixture(None, false);

        let tree_reader = build_hierarchy(0);

        let mut snapshot = InspectionSnapshot::default();
        let mut snapshot_budget = SnapshotBudget::new(4);

        inspect_folder_types_hive(
            &folder_reader,
            RootKey::Hkcu,
            TEST_FOLDER_TYPES,
            ObservationSource::HkcuFolderTypes,
            &mut snapshot.hkcu_folder_types,
            MAX_VALUE_BYTES,
            &mut snapshot_budget,
            &mut TraversalBudget::new(MAX_KEYS_PER_DOMAIN),
        );

        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);
        inspect_key_tree(
            &tree_reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
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

        let bags_root = snapshot.bags.get("root").expect("bags root missing");
        assert!(!bags_root.values.contains_key("TestValue"));

        let errs: Vec<_> = bags_root
            .read_errors
            .iter()
            .filter(|e| e.operation == "snapshot_size_limit")
            .collect();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].root, RootKey::Hkcu);
        assert_eq!(errs[0].key_path, "root");
        assert!(errs[0].error.contains("TestValue"));

        assert_eq!(snapshot_budget.captured_bytes, 4);

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn nested_open_key_failure_produces_partial_status() {
        let mut data = BTreeMap::new();

        data.insert("root".to_string(), vec!["missing_child".to_string()]);

        // Deliberately do not register the child path.
        let reader = FakeReader::new(data);

        let mut snapshot = InspectionSnapshot::default();
        let mut budget = TraversalBudget::new(MAX_KEYS_PER_DOMAIN);

        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        inspect_key_tree(
            &reader,
            RootKey::Hkcu,
            "root",
            ObservationSource::HkcuBags,
            &mut snapshot.bags,
            "",
            KeyPresence::Required,
            0,
            &mut budget,
            &mut snapshot_budget,
        );

        finalize_inspection_status(&mut snapshot);

        let root = snapshot.bags.get("root").expect("root missing");

        assert!(matches!(
            root.values.get("TestValue"),
            Some(ValueReadResult::Success(_))
        ));

        let child = root
            .subkeys
            .get(r"root\missing_child")
            .expect("child error snapshot missing");

        assert_eq!(child.read_errors.len(), 1);
        assert_eq!(child.read_errors[0].operation, "open_key");

        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn key_error_count_includes_all_depths() {
        let mut root = KeySnapshot::default();
        root.read_errors.push(InspectionError {
            key_path: "root".to_string(),
            root: RootKey::Hkcu,
            operation: "open_key".to_string(),
            error: "boom".to_string(),
        });

        let mut child1 = KeySnapshot::default();
        child1.read_errors.push(InspectionError {
            key_path: r"root\child1".to_string(),
            root: RootKey::Hkcu,
            operation: "open_key".to_string(),
            error: "boom".to_string(),
        });
        let mut child2 = KeySnapshot::default();
        child2.read_errors.push(InspectionError {
            key_path: r"root\child2".to_string(),
            root: RootKey::Hkcu,
            operation: "open_key".to_string(),
            error: "boom".to_string(),
        });

        let mut grandchild = KeySnapshot::default();
        grandchild.read_errors.push(InspectionError {
            key_path: r"root\child1\grandchild".to_string(),
            root: RootKey::Hkcu,
            operation: "open_key".to_string(),
            error: "boom".to_string(),
        });
        child1
            .subkeys
            .insert(r"root\child1\grandchild".to_string(), grandchild);

        root.subkeys.insert(r"root\child1".to_string(), child1);
        root.subkeys.insert(r"root\child2".to_string(), child2);

        assert_eq!(count_key_read_errors(&root), 4);
    }

    #[test]
    fn folder_types_error_count_includes_nested_records() {
        let mut folder_types = FolderTypesSnapshot::default();
        folder_types.read_errors.push(InspectionError {
            key_path: "FolderTypes".to_string(),
            root: RootKey::Hkcu,
            operation: "open_key".to_string(),
            error: "boom".to_string(),
        });

        let mut folder = FolderTypeSnapshot {
            guid: "{00000000-0000-0000-0000-000000000000}".to_string(),
            canonical_name: None,
            top_views: BTreeMap::new(),
            read_errors: Vec::new(),
        };
        folder.read_errors.push(InspectionError {
            key_path: r"FolderTypes\{00000000-0000-0000-0000-000000000000}".to_string(),
            root: RootKey::Hkcu,
            operation: "open_key".to_string(),
            error: "boom".to_string(),
        });
        folder.read_errors.push(InspectionError {
            key_path: r"FolderTypes\{00000000-0000-0000-0000-000000000000}\TopViews\{00000000-0000-0000-0000-000000000000}".to_string(),
            root: RootKey::Hkcu,
            operation: "open_key".to_string(),
            error: "boom".to_string(),
        });

        folder_types
            .types
            .insert("{00000000-0000-0000-0000-000000000000}".to_string(), folder);

        assert_eq!(count_folder_types_read_errors(&folder_types), 3);
    }

    #[test]
    fn snapshot_error_count_covers_all_domains() {
        let mut snapshot = InspectionSnapshot::default();

        let err = |key_path: &str, operation: &str| InspectionError {
            key_path: key_path.to_string(),
            root: RootKey::Hkcu,
            operation: operation.to_string(),
            error: "boom".to_string(),
        };

        // snapshot.errors
        snapshot.errors.push(err("snapshot", "open_key"));

        // HKCU FolderTypes
        snapshot
            .hkcu_folder_types
            .read_errors
            .push(err("HKCU\\FolderTypes", "open_key"));

        // HKLM FolderTypes
        snapshot
            .hklm_folder_types
            .read_errors
            .push(err("HKLM\\FolderTypes", "open_key"));

        // Advanced
        snapshot
            .advanced
            .read_errors
            .push(err("Advanced", "open_key"));

        // Bags — nested child error
        let mut bags_root = KeySnapshot::default();
        let mut bags_child = KeySnapshot::default();
        bags_child
            .read_errors
            .push(err(r"Bags\root\child", "open_key"));
        bags_root
            .subkeys
            .insert(r"Bags\root\child".to_string(), bags_child);
        snapshot.bags.insert("Bags\\root".to_string(), bags_root);

        // Streams — nested child error
        let mut streams_root = KeySnapshot::default();
        let mut streams_child = KeySnapshot::default();
        streams_child
            .read_errors
            .push(err(r"Streams\root\child", "open_key"));
        streams_root
            .subkeys
            .insert(r"Streams\root\child".to_string(), streams_child);
        snapshot
            .streams
            .insert("Streams\\root".to_string(), streams_root);

        // CLSID — nested child error
        let mut clsid_root = KeySnapshot::default();
        let mut clsid_child = KeySnapshot::default();
        clsid_child
            .read_errors
            .push(err(r"CLSID\root\child", "open_key"));
        clsid_root
            .subkeys
            .insert(r"CLSID\root\child".to_string(), clsid_child);
        snapshot.clsid.insert("CLSID\\root".to_string(), clsid_root);

        // Policies — nested child error
        let mut policies_root = KeySnapshot::default();
        let mut policies_child = KeySnapshot::default();
        policies_child
            .read_errors
            .push(err(r"Policies\root\child", "open_key"));
        policies_root
            .subkeys
            .insert(r"Policies\root\child".to_string(), policies_child);
        snapshot
            .policies
            .insert("Policies\\root".to_string(), policies_root);

        assert_eq!(count_snapshot_read_errors(&snapshot), 8);

        finalize_inspection_status(&mut snapshot);

        assert!(matches!(
            snapshot.status,
            InspectionStatus::Failed { error }
                if error == "Inspection failed with 8 errors"
        ));
    }

    #[test]
    fn no_recorded_errors_remains_complete() {
        let mut snapshot = InspectionSnapshot::default();

        assert_eq!(count_snapshot_read_errors(&snapshot), 0);

        finalize_inspection_status(&mut snapshot);

        assert!(matches!(snapshot.status, InspectionStatus::Complete));
    }

    #[test]
    fn value_enumeration_failure_preserves_child_traversal() {
        let reader = build_hierarchy(1).with_enum_failures(&["root"], &[]);

        let (snapshot, _) = inspect_test_tree(&reader, MAX_KEYS_PER_DOMAIN);

        let root = snapshot.bags.get("root").expect("root missing");

        assert!(root.values.is_empty());
        assert_eq!(root.read_errors.len(), 1);

        let error = &root.read_errors[0];

        assert_eq!(error.operation, "enum_values");
        assert_eq!(error.root, RootKey::Hkcu);
        assert_eq!(error.key_path, "root");

        assert!(error.error.contains("injected value enumeration failure"));

        let child = root.subkeys.get(r"root\child").expect("child missing");

        assert!(matches!(
            child.values.get("TestValue"),
            Some(ValueReadResult::Success(_))
        ));

        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn subkey_enumeration_failure_preserves_values() {
        let reader = build_hierarchy(1).with_enum_failures(&[], &["root"]);

        let (snapshot, _) = inspect_test_tree(&reader, MAX_KEYS_PER_DOMAIN);

        let root = snapshot.bags.get("root").expect("root missing");

        let value = root.values.get("TestValue").expect("TestValue missing");
        match value {
            ValueReadResult::Success(r) => {
                assert_eq!(r.raw_bytes, vec![42u8, 0, 0, 0]);
                assert!(matches!(r.decoded.as_ref(), Some(RegistryValue::Dword(42))));
            }
            _ => panic!("expected successful value read"),
        }

        assert!(root.subkeys.is_empty());

        assert_eq!(root.read_errors.len(), 1);

        let error = &root.read_errors[0];

        assert_eq!(error.operation, "enum_subkeys");
        assert_eq!(error.root, RootKey::Hkcu);
        assert_eq!(error.key_path, "root");

        assert!(error.error.contains("injected subkey enumeration failure"));

        let opened = reader.opened_keys.lock().unwrap();
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0], (RootKey::Hkcu, "root".to_string()));

        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn nested_enumeration_failure_produces_partial_status() {
        let reader = build_hierarchy(2).with_enum_failures(&[r"root\child"], &[]);

        let (snapshot, _) = inspect_test_tree(&reader, MAX_KEYS_PER_DOMAIN);

        let root = snapshot.bags.get("root").expect("root missing");

        assert!(matches!(
            root.values.get("TestValue"),
            Some(ValueReadResult::Success(_))
        ));

        let child = root.subkeys.get(r"root\child").expect("child missing");

        assert_eq!(child.read_errors.len(), 1);

        let error = &child.read_errors[0];

        assert_eq!(error.operation, "enum_values");
        assert_eq!(error.root, RootKey::Hkcu);
        assert_eq!(error.key_path, r"root\child");

        assert!(error.error.contains("injected value enumeration failure"));

        let grandchild = root
            .subkeys
            .get(r"root\child")
            .and_then(|k| k.subkeys.get(r"root\child\child"))
            .expect("grandchild missing");

        assert!(matches!(
            grandchild.values.get("TestValue"),
            Some(ValueReadResult::Success(_))
        ));

        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn both_enumeration_failures_are_recorded() {
        let reader = build_hierarchy(0).with_enum_failures(&["root"], &["root"]);

        let (snapshot, _) = inspect_test_tree(&reader, MAX_KEYS_PER_DOMAIN);

        let root = snapshot.bags.get("root").expect("root missing");

        assert!(root.values.is_empty());

        assert_eq!(root.read_errors.len(), 2);

        let enum_values_err = root
            .read_errors
            .iter()
            .find(|e| e.operation == "enum_values")
            .expect("enum_values error missing");
        assert_eq!(enum_values_err.root, RootKey::Hkcu);
        assert_eq!(enum_values_err.key_path, "root");
        assert!(enum_values_err
            .error
            .contains("injected value enumeration failure"));

        let enum_subkeys_err = root
            .read_errors
            .iter()
            .find(|e| e.operation == "enum_subkeys")
            .expect("enum_subkeys error missing");
        assert_eq!(enum_subkeys_err.root, RootKey::Hkcu);
        assert_eq!(enum_subkeys_err.key_path, "root");
        assert!(enum_subkeys_err
            .error
            .contains("injected subkey enumeration failure"));

        assert_eq!(count_snapshot_read_errors(&snapshot), 2);

        assert!(matches!(snapshot.status, InspectionStatus::Failed { .. }));
    }

    #[test]
    fn folder_types_enumeration_failure_is_reported() {
        let reader =
            build_folder_types_fixture(None, false).with_enum_failures(&[], &[TEST_FOLDER_TYPES]);

        let mut snapshot = InspectionSnapshot::default();
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        inspect_folder_types_hive(
            &reader,
            RootKey::Hkcu,
            TEST_FOLDER_TYPES,
            ObservationSource::HkcuFolderTypes,
            &mut snapshot.hkcu_folder_types,
            MAX_VALUE_BYTES,
            &mut snapshot_budget,
            &mut TraversalBudget::new(MAX_KEYS_PER_DOMAIN),
        );

        finalize_inspection_status(&mut snapshot);

        assert_eq!(snapshot.hkcu_folder_types.read_errors.len(), 1);

        let error = &snapshot.hkcu_folder_types.read_errors[0];

        assert_eq!(error.operation, "enum_subkeys");
        assert_eq!(error.root, RootKey::Hkcu);
        assert_eq!(error.key_path, TEST_FOLDER_TYPES);
        assert!(error.error.contains("injected subkey enumeration failure"));

        assert!(snapshot.hkcu_folder_types.types.is_empty());

        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert!(matches!(snapshot.status, InspectionStatus::Failed { .. }));
    }

    #[test]
    fn windows_missing_key_error_is_classified() {
        let error = classify_windows_error(
            0x80070002,
            RootKey::Hkcu,
            "Software\\Test",
            None,
            "The system cannot find the file specified.".to_string(),
        );

        assert!(matches!(
            error,
            RegistryError::KeyNotFound { root, path }
                if root == RootKey::Hkcu && path == "Software\\Test"
        ));
    }

    #[test]
    fn windows_missing_value_error_is_classified() {
        let error = classify_windows_error(
            0x80070002,
            RootKey::Hkcu,
            "Software\\Test",
            Some("Mode"),
            "The system cannot find the file specified.".to_string(),
        );

        assert!(matches!(
            error,
            RegistryError::ValueNotFound { root, path, name }
                if root == RootKey::Hkcu
                    && path == "Software\\Test"
                    && name == "Mode"
        ));
    }

    #[test]
    fn windows_missing_path_is_not_missing_value() {
        let error = classify_windows_error(
            0x80070003,
            RootKey::Hkcu,
            "Software\\Test",
            Some("Mode"),
            "The system cannot find the path specified.".to_string(),
        );

        assert!(matches!(
            error,
            RegistryError::KeyNotFound { root, path }
                if root == RootKey::Hkcu && path == "Software\\Test"
        ));
    }

    #[test]
    fn windows_access_denied_error_is_classified() {
        let error = classify_windows_error(
            0x80070005,
            RootKey::Hklm,
            "Software\\Policies",
            None,
            "Access is denied.".to_string(),
        );

        assert!(matches!(
            error,
            RegistryError::AccessDenied { root, path }
                if root == RootKey::Hklm && path == "Software\\Policies"
        ));
    }

    #[test]
    fn windows_unknown_error_preserves_diagnostics() {
        let error = classify_windows_error(
            0xC0000001,
            RootKey::Hkcu,
            "Software\\Test",
            None,
            "Custom diagnostic message".to_string(),
        );

        assert!(matches!(
            error,
            RegistryError::WindowsApi(msg)
                if msg == "Custom diagnostic message"
        ));
    }

    #[test]
    fn windows_error_classification_is_language_independent() {
        let english = classify_windows_error(
            0x80070002,
            RootKey::Hkcu,
            "Software\\Test",
            Some("Mode"),
            "The system cannot find the file specified.".to_string(),
        );
        let non_english = classify_windows_error(
            0x80070002,
            RootKey::Hkcu,
            "Software\\Test",
            Some("Mode"),
            "Le système ne peut pas trouver le fichier spécifié.".to_string(),
        );

        assert!(matches!(
            english,
            RegistryError::ValueNotFound { name, .. } if name == "Mode"
        ));
        assert!(matches!(
            non_english,
            RegistryError::ValueNotFound { name, .. } if name == "Mode"
        ));

        // An unrelated numeric error code containing the English phrase
        // "not found" must remain WindowsApi, not KeyNotFound.
        let unrelated = classify_windows_error(
            0xC0000001,
            RootKey::Hkcu,
            "Software\\Test",
            None,
            "object name not found".to_string(),
        );
        assert!(matches!(
            unrelated,
            RegistryError::WindowsApi(msg) if msg == "object name not found"
        ));
    }

    #[test]
    fn recursive_access_denied_value_is_reported() {
        let reader = build_hierarchy(1).with_value_failure(
            "root",
            "TestValue",
            InjectedValueFailure::AccessDenied,
        );

        let (snapshot, _) = inspect_test_tree(&reader, MAX_KEYS_PER_DOMAIN);

        let root = snapshot.bags.get("root").expect("root missing");

        assert!(matches!(
            root.values.get("TestValue"),
            Some(ValueReadResult::Failure(ReadError::AccessDenied { .. }))
        ));

        assert_eq!(root.read_errors.len(), 1);

        let error = &root.read_errors[0];

        assert_eq!(error.operation, "get_value");
        assert_eq!(error.root, RootKey::Hkcu);
        assert_eq!(error.key_path, "root");
        assert!(error.error.contains("TestValue"));

        let child = root.subkeys.get(r"root\child").expect("child missing");

        assert!(matches!(
            child.values.get("TestValue"),
            Some(ValueReadResult::Success(_))
        ));

        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn enumerated_missing_value_is_reported() {
        let reader = build_hierarchy(0).with_value_failure(
            "root",
            "TestValue",
            InjectedValueFailure::ValueNotFound,
        );

        let (snapshot, _) = inspect_test_tree(&reader, MAX_KEYS_PER_DOMAIN);

        let root = snapshot.bags.get("root").expect("root missing");

        assert!(matches!(
            root.values.get("TestValue"),
            Some(ValueReadResult::Failure(ReadError::ValueNotFound { .. }))
        ));

        assert_eq!(root.read_errors.len(), 1);

        let error = &root.read_errors[0];

        assert_eq!(error.operation, "get_value");
        assert_eq!(error.root, RootKey::Hkcu);
        assert_eq!(error.key_path, "root");

        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert!(matches!(snapshot.status, InspectionStatus::Failed { .. }));
    }

    #[test]
    fn advanced_missing_optional_values_are_not_errors() {
        use winview_core::registry::keys::advanced_values::HIDE_FILE_EXT;
        use winview_core::registry::keys::advanced_values::USE_COMPACT_MODE;
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
        inspect_advanced(
            &reader,
            &mut snapshot,
            MAX_VALUE_BYTES,
            &mut snapshot_budget,
        );
        finalize_inspection_status(&mut snapshot);

        let hide_value = snapshot
            .advanced
            .values
            .get(HIDE_FILE_EXT)
            .expect("HideFileExt missing");
        match hide_value {
            ValueReadResult::Success(r) => {
                assert_eq!(r.raw_bytes, vec![42u8, 0, 0, 0]);
                assert!(matches!(r.decoded.as_ref(), Some(RegistryValue::Dword(42))));
            }
            _ => panic!("expected successful HideFileExt read"),
        }

        let compact_value = snapshot
            .advanced
            .values
            .get(USE_COMPACT_MODE)
            .expect("UseCompactMode missing");
        assert!(matches!(
            compact_value,
            ValueReadResult::Failure(ReadError::ValueNotFound { .. })
        ));

        assert_eq!(snapshot.advanced.read_errors.len(), 0);

        assert_eq!(count_snapshot_read_errors(&snapshot), 0);

        assert!(matches!(snapshot.status, InspectionStatus::Complete));
    }

    #[test]
    fn advanced_access_denied_is_reported() {
        use winview_core::registry::keys::advanced_values::AUTO_CHECK_SELECT;
        use winview_core::registry::keys::advanced_values::HIDE_FILE_EXT;
        use winview_core::registry::keys::advanced_values::USE_COMPACT_MODE;
        use winview_core::registry::keys::paths::ADVANCED;

        let mut data = BTreeMap::new();
        data.insert(ADVANCED.to_string(), vec![]);

        let mut values = BTreeMap::new();
        let mut adv_values = BTreeMap::new();
        adv_values.insert(
            USE_COMPACT_MODE.to_string(),
            (
                4,
                42u32.to_le_bytes().to_vec(),
                Some(RegistryValue::Dword(42)),
                DecodingStatus::Decoded,
            ),
        );
        values.insert(ADVANCED.to_string(), adv_values);

        let reader = FakeReader::with_values(data, values).with_value_failure(
            ADVANCED,
            HIDE_FILE_EXT,
            InjectedValueFailure::AccessDenied,
        );

        let mut snapshot = InspectionSnapshot::default();
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);
        inspect_advanced(
            &reader,
            &mut snapshot,
            MAX_VALUE_BYTES,
            &mut snapshot_budget,
        );
        finalize_inspection_status(&mut snapshot);

        let hide_value = snapshot
            .advanced
            .values
            .get(HIDE_FILE_EXT)
            .expect("HideFileExt missing");
        assert!(matches!(
            hide_value,
            ValueReadResult::Failure(ReadError::AccessDenied { .. })
        ));

        let compact_value = snapshot
            .advanced
            .values
            .get(USE_COMPACT_MODE)
            .expect("UseCompactMode missing");
        match compact_value {
            ValueReadResult::Success(r) => {
                assert_eq!(r.raw_bytes, vec![42u8, 0, 0, 0]);
                assert!(matches!(r.decoded.as_ref(), Some(RegistryValue::Dword(42))));
            }
            _ => panic!("expected successful UseCompactMode read"),
        }

        // Other absent optional settings (AUTO_CHECK_SELECT etc.) should not
        // produce inspection errors since they are ValueNotFound + optional.
        let absent_value = snapshot
            .advanced
            .values
            .get(AUTO_CHECK_SELECT)
            .expect("AUTO_CHECK_SELECT missing");
        assert!(matches!(
            absent_value,
            ValueReadResult::Failure(ReadError::ValueNotFound { .. })
        ));

        assert_eq!(snapshot.advanced.read_errors.len(), 1);

        let error = &snapshot.advanced.read_errors[0];

        assert_eq!(error.operation, "get_value");
        assert_eq!(error.root, RootKey::Hkcu);
        assert_eq!(error.key_path, ADVANCED);
        assert!(error.error.contains(HIDE_FILE_EXT));

        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn folder_type_value_access_denied_is_reported() {
        let reader = build_folder_types_fixture(None, false).with_value_failure(
            &format!(
                r"{}\{}\TopViews\{}",
                TEST_FOLDER_TYPES, TEST_FOLDER_GUID, TEST_VIEW_GUID
            ),
            "Mode",
            InjectedValueFailure::AccessDenied,
        );

        let mut snapshot = InspectionSnapshot::default();
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        inspect_folder_types_hive(
            &reader,
            RootKey::Hkcu,
            TEST_FOLDER_TYPES,
            ObservationSource::HkcuFolderTypes,
            &mut snapshot.hkcu_folder_types,
            MAX_VALUE_BYTES,
            &mut snapshot_budget,
            &mut TraversalBudget::new(MAX_KEYS_PER_DOMAIN),
        );

        finalize_inspection_status(&mut snapshot);

        let folder = snapshot
            .hkcu_folder_types
            .types
            .get(TEST_FOLDER_GUID)
            .expect("folder type missing");

        let view = folder.top_views.get(TEST_VIEW_GUID).expect("view missing");

        assert!(matches!(
            view.raw_values.get("Mode"),
            Some(ValueReadResult::Failure(ReadError::AccessDenied { .. }))
        ));
        assert_eq!(view.mode, None);

        // Other missing optional settings should not be counted as errors.
        assert!(view.logical_view_mode.is_none());
        assert!(view.icon_size.is_none());

        assert_eq!(snapshot.hkcu_folder_types.read_errors.len(), 1);

        let error = &snapshot.hkcu_folder_types.read_errors[0];

        assert_eq!(error.operation, "get_value");
        assert_eq!(error.root, RootKey::Hkcu);
        assert_eq!(
            error.key_path,
            format!(
                r"{}\{}\TopViews\{}",
                TEST_FOLDER_TYPES, TEST_FOLDER_GUID, TEST_VIEW_GUID
            )
        );
        assert!(error.error.contains("Mode"));

        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert!(matches!(snapshot.status, InspectionStatus::Failed { .. }));
    }

    #[test]
    fn canonical_name_access_denied_preserves_view_data() {
        use winview_core::registry::keys::paths::*;

        let reader = build_folder_types_fixture(Some("GeneralItems"), false).with_value_failure(
            &format!(r"{}\{}", TEST_FOLDER_TYPES, TEST_FOLDER_GUID),
            CANONICAL_NAME,
            InjectedValueFailure::AccessDenied,
        );

        let mut snapshot = InspectionSnapshot::default();
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        inspect_folder_types_hive(
            &reader,
            RootKey::Hkcu,
            TEST_FOLDER_TYPES,
            ObservationSource::HkcuFolderTypes,
            &mut snapshot.hkcu_folder_types,
            MAX_VALUE_BYTES,
            &mut snapshot_budget,
            &mut TraversalBudget::new(MAX_KEYS_PER_DOMAIN),
        );

        finalize_inspection_status(&mut snapshot);

        let folder = snapshot
            .hkcu_folder_types
            .types
            .get(TEST_FOLDER_GUID)
            .expect("folder type missing");

        assert_eq!(folder.canonical_name, None);

        let view = folder.top_views.get(TEST_VIEW_GUID).expect("view missing");

        assert_eq!(view.mode, Some(42));

        assert_eq!(snapshot.hkcu_folder_types.read_errors.len(), 1);

        let error = &snapshot.hkcu_folder_types.read_errors[0];

        assert_eq!(error.operation, "get_value");
        assert_eq!(error.root, RootKey::Hkcu);
        assert_eq!(
            error.key_path,
            format!(r"{}\{}", TEST_FOLDER_TYPES, TEST_FOLDER_GUID)
        );
        assert!(error.error.contains(CANONICAL_NAME));

        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn failed_key_open_without_observations_is_failed() {
        let reader = FakeReader::new(BTreeMap::new());

        let (snapshot, _) = inspect_test_tree(&reader, MAX_KEYS_PER_DOMAIN);

        let root = snapshot
            .bags
            .get("root")
            .expect("root error snapshot missing");

        assert_eq!(root.read_errors.len(), 1);
        assert_eq!(root.read_errors[0].operation, "open_key");

        assert!(!snapshot_has_successful_observations(&snapshot));
        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert!(matches!(
            snapshot.status,
            InspectionStatus::Failed { error }
                if error == "Inspection failed with 1 errors"
        ));
    }

    #[test]
    fn nested_successful_observation_produces_partial_status() {
        let reader = build_hierarchy(1).with_value_failure(
            "root",
            "TestValue",
            InjectedValueFailure::AccessDenied,
        );

        let (snapshot, _) = inspect_test_tree(&reader, MAX_KEYS_PER_DOMAIN);

        let root = snapshot.bags.get("root").expect("root missing");

        assert!(matches!(
            root.values.get("TestValue"),
            Some(ValueReadResult::Failure(ReadError::AccessDenied { .. }))
        ));

        let child = root.subkeys.get(r"root\child").expect("child missing");

        assert!(matches!(
            child.values.get("TestValue"),
            Some(ValueReadResult::Success(_))
        ));

        assert!(snapshot_has_successful_observations(&snapshot));
        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn advanced_optional_absence_without_errors_is_complete() {
        use winview_core::registry::keys::paths::ADVANCED;
        use winview_core::test_support::{FakeRegistryKey, FakeRegistryReader};

        let reader = FakeRegistryReader::new();

        reader.insert_key(RootKey::Hkcu, ADVANCED, FakeRegistryKey::new());

        let mut snapshot = InspectionSnapshot::default();
        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        inspect_advanced(
            &reader,
            &mut snapshot,
            MAX_VALUE_BYTES,
            &mut snapshot_budget,
        );

        finalize_inspection_status(&mut snapshot);

        assert_eq!(snapshot.advanced.values.len(), 7);

        assert!(snapshot.advanced.values.values().all(|value| matches!(
            value,
            ValueReadResult::Failure(ReadError::ValueNotFound { .. })
        )));

        assert!(!snapshot_has_successful_observations(&snapshot));
        assert_eq!(count_snapshot_read_errors(&snapshot), 0);

        assert!(matches!(snapshot.status, InspectionStatus::Complete));
    }

    #[test]
    fn undecoded_raw_observation_counts_as_success() {
        let cases = [
            (0u32, vec![1u8, 2, 3], DecodingStatus::UnknownType),
            (
                4u32,
                vec![1u8, 2],
                DecodingStatus::Malformed {
                    reason: "invalid DWORD length".into(),
                },
            ),
            (0u32, Vec::new(), DecodingStatus::UnknownType),
        ];

        for (type_id, bytes, status) in cases {
            let mut data = BTreeMap::new();
            data.insert("root".to_string(), Vec::new());

            let mut values = FakeValues::new();

            values.entry("root".to_string()).or_default().insert(
                "TestValue".to_string(),
                (
                    type_id,
                    bytes.clone(),
                    Some(RegistryValue::Binary(bytes.clone())),
                    status.clone(),
                ),
            );

            let reader = FakeReader::with_values(data, values);

            let (mut snapshot, _) = inspect_test_tree(&reader, MAX_KEYS_PER_DOMAIN);

            let root = snapshot.bags.get("root").expect("root missing");

            match root.values.get("TestValue") {
                Some(ValueReadResult::Success(raw)) => {
                    assert_eq!(raw.raw_bytes, bytes);
                    assert_eq!(raw.decoded, None);
                    assert_eq!(raw.decoding_status, status);
                }
                _ => panic!("expected successful raw observation"),
            }

            assert!(snapshot_has_successful_observations(&snapshot));

            snapshot.errors.push(InspectionError {
                key_path: "other".into(),
                root: RootKey::Hkcu,
                operation: "open_key".into(),
                error: "injected failure".into(),
            });

            finalize_inspection_status(&mut snapshot);

            assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
        }
    }

    #[test]
    fn canonical_name_only_counts_as_success() {
        let view_path = format!(
            r"{}\{}\TopViews\{}",
            TEST_FOLDER_TYPES, TEST_FOLDER_GUID, TEST_VIEW_GUID,
        );

        let reader = build_folder_types_fixture(Some("A"), false).with_value_failure(
            &view_path,
            "Mode",
            InjectedValueFailure::AccessDenied,
        );

        let mut snapshot = InspectionSnapshot::default();

        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        inspect_folder_types_hive(
            &reader,
            RootKey::Hkcu,
            TEST_FOLDER_TYPES,
            ObservationSource::HkcuFolderTypes,
            &mut snapshot.hkcu_folder_types,
            MAX_VALUE_BYTES,
            &mut snapshot_budget,
            &mut TraversalBudget::new(MAX_KEYS_PER_DOMAIN),
        );

        finalize_inspection_status(&mut snapshot);

        let folder = snapshot
            .hkcu_folder_types
            .types
            .get(TEST_FOLDER_GUID)
            .expect("folder missing");

        assert_eq!(folder.canonical_name.as_deref(), Some("A"));

        let view = folder.top_views.get(TEST_VIEW_GUID).expect("view missing");

        assert!(matches!(
            view.raw_values.get("Mode"),
            Some(ValueReadResult::Failure(ReadError::AccessDenied { .. }))
        ));

        assert!(!view
            .raw_values
            .values()
            .any(|value| matches!(value, ValueReadResult::Success(_))));

        assert!(snapshot_has_successful_observations(&snapshot));
        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    fn utf16_units_to_bytes(units: &[u16]) -> Vec<u8> {
        units.iter().flat_map(|unit| unit.to_le_bytes()).collect()
    }

    #[test]
    fn multi_sz_empty_sequences() {
        for units in [vec![], vec![0], vec![0, 0]] {
            let bytes = utf16_units_to_bytes(&units);

            assert_eq!(decode_reg_multi_sz(&bytes).unwrap(), Vec::<String>::new());
        }
    }

    #[test]
    fn multi_sz_single_element() {
        let bytes = utf16_units_to_bytes(&[65, 0, 0]);

        assert_eq!(decode_reg_multi_sz(&bytes).unwrap(), vec!["A".to_string()]);
    }

    #[test]
    fn multi_sz_multiple_unicode_elements() {
        let mut units: Vec<u16> = "First".encode_utf16().collect();

        units.push(0);
        units.extend("안녕하세요".encode_utf16());
        units.push(0);
        units.extend("😀".encode_utf16());
        units.extend([0, 0]);

        let bytes = utf16_units_to_bytes(&units);

        assert_eq!(
            decode_reg_multi_sz(&bytes).unwrap(),
            vec![
                "First".to_string(),
                "안녕하세요".to_string(),
                "😀".to_string(),
            ]
        );
    }

    #[test]
    fn multi_sz_rejects_missing_terminator() {
        for units in [vec![65], vec![65, 0], vec![65, 0, 66, 0]] {
            let bytes = utf16_units_to_bytes(&units);

            assert!(decode_reg_multi_sz(&bytes).is_err());
        }
    }

    #[test]
    fn multi_sz_rejects_invalid_encoding() {
        // Odd byte length.
        assert!(decode_reg_multi_sz(&[65, 0, 0]).is_err());

        // Unpaired UTF-16 high surrogate.
        let invalid_high = utf16_units_to_bytes(&[0xD800, 0, 0]);

        assert!(decode_reg_multi_sz(&invalid_high).is_err());

        // Unpaired UTF-16 low surrogate.
        let invalid_low = utf16_units_to_bytes(&[0xDC00, 0, 0]);

        assert!(decode_reg_multi_sz(&invalid_low).is_err());
    }

    #[test]
    fn multi_sz_stops_at_empty_element() {
        let bytes = utf16_units_to_bytes(&[65, 0, 0, 66, 0, 0]);

        assert_eq!(decode_reg_multi_sz(&bytes).unwrap(), vec!["A".to_string()]);
    }

    #[test]
    fn multi_sz_conversion_preserves_raw_bytes() {
        let valid = utf16_units_to_bytes(&[65, 0, 66, 0, 0]);

        let (type_name, decoded, raw, status) =
            convert_registry_value(RegType::MultiString, &valid);

        assert_eq!(type_name, ValueTypeName::RegMultiSz);
        assert_eq!(raw, valid);

        assert_eq!(
            decoded,
            RegistryValue::MultiString(vec!["A".to_string(), "B".to_string(),])
        );

        assert_eq!(status, DecodingStatus::Decoded);

        let malformed = utf16_units_to_bytes(&[65, 0]);

        let (type_name, fallback, raw, status) =
            convert_registry_value(RegType::MultiString, &malformed);

        assert_eq!(type_name, ValueTypeName::RegMultiSz);
        assert_eq!(raw, malformed);

        assert_eq!(fallback, RegistryValue::Binary(malformed));

        assert!(matches!(status, DecodingStatus::Malformed { .. }));
    }

    #[test]
    fn utf16_decodes_null_terminated_strings() {
        let mut units: Vec<u16> = "GeneralItems".encode_utf16().collect();

        units.push(0);

        let bytes = utf16_units_to_bytes(&units);

        assert_eq!(decode_utf16_le(&bytes).unwrap(), "GeneralItems");
    }

    #[test]
    fn utf16_decodes_unicode_and_surrogate_pairs() {
        for text in ["안녕하세요", "こんにちは", "你好", "Explorer 😀", "Café"] {
            let mut units: Vec<u16> = text.encode_utf16().collect();

            units.push(0);

            let bytes = utf16_units_to_bytes(&units);

            assert_eq!(decode_utf16_le(&bytes).unwrap(), text);
        }
    }

    #[test]
    fn utf16_accepts_empty_and_unterminated_strings() {
        assert_eq!(decode_utf16_le(&[]).unwrap(), "");

        let empty = utf16_units_to_bytes(&[0]);

        assert_eq!(decode_utf16_le(&empty).unwrap(), "");

        let unterminated = utf16_units_to_bytes(&[65, 66, 67]);

        assert_eq!(decode_utf16_le(&unterminated).unwrap(), "ABC");
    }

    #[test]
    fn utf16_stops_at_first_null() {
        let bytes = utf16_units_to_bytes(&[65, 66, 0, 67, 68, 0]);

        assert_eq!(decode_utf16_le(&bytes).unwrap(), "AB");

        let starts_with_null = utf16_units_to_bytes(&[0, 65, 66]);

        assert_eq!(decode_utf16_le(&starts_with_null).unwrap(), "");
    }

    #[test]
    fn utf16_rejects_invalid_encoding() {
        assert!(decode_utf16_le(&[65]).is_err());

        assert!(decode_utf16_le(&[65, 0, 66]).is_err());

        for units in [
            vec![0xD800, 0],
            vec![0xDC00, 0],
            vec![0xD800, 65, 0],
            vec![65, 0xDC00, 0],
        ] {
            let bytes = utf16_units_to_bytes(&units);

            assert!(decode_utf16_le(&bytes).is_err());
        }
    }

    #[test]
    fn reg_sz_conversion_preserves_raw_bytes() {
        let bytes = utf16_units_to_bytes(&[65, 66, 0, 67, 68, 0]);

        let (type_name, decoded, raw, status) = convert_registry_value(RegType::String, &bytes);

        assert_eq!(type_name, ValueTypeName::RegSz);

        assert_eq!(type_name.to_raw_id(), 1);

        assert_eq!(decoded, RegistryValue::String("AB".into()));

        assert_eq!(raw, bytes);

        assert_eq!(status, DecodingStatus::Decoded);
    }

    #[test]
    fn reg_expand_sz_preserves_environment_variables() {
        let text = r"%USERPROFILE%\Documents";

        let mut units: Vec<u16> = text.encode_utf16().collect();

        units.push(0);

        let bytes = utf16_units_to_bytes(&units);

        let (type_name, decoded, raw, status) =
            convert_registry_value(RegType::ExpandString, &bytes);

        assert_eq!(type_name, ValueTypeName::RegExpandSz);

        assert_eq!(type_name.to_raw_id(), 2);

        assert_eq!(decoded, RegistryValue::ExpandString(text.to_string()));

        assert_eq!(raw, bytes);

        assert_eq!(status, DecodingStatus::Decoded);
    }

    #[test]
    fn malformed_registry_strings_preserve_raw_bytes() {
        let malformed_payloads = [
            vec![65, 0, 66],
            utf16_units_to_bytes(&[0xD800, 0]),
            utf16_units_to_bytes(&[0xDC00, 0]),
        ];

        for bytes in malformed_payloads {
            for (reg_type, expected_type) in [
                (RegType::String, ValueTypeName::RegSz),
                (RegType::ExpandString, ValueTypeName::RegExpandSz),
            ] {
                let (type_name, fallback, raw, status) = convert_registry_value(reg_type, &bytes);

                assert_eq!(type_name, expected_type);

                assert_eq!(raw, bytes);

                assert_eq!(fallback, RegistryValue::Binary(bytes.clone()));

                assert!(matches!(status, DecodingStatus::Malformed { .. }));
            }
        }
    }

    #[test]
    fn reg_dword_decodes_little_endian_values() {
        for value in [0u32, 1, 42, 0x12345678, u32::MAX] {
            let bytes = value.to_le_bytes();

            let (type_name, decoded, raw, status) = convert_registry_value(RegType::U32, &bytes);

            assert_eq!(type_name, ValueTypeName::RegDword);

            assert_eq!(type_name.to_raw_id(), 4);

            assert_eq!(decoded, RegistryValue::Dword(value));

            assert_eq!(raw, bytes.to_vec());

            assert_eq!(status, DecodingStatus::Decoded);
        }
    }

    #[test]
    fn reg_dword_rejects_invalid_lengths() {
        for length in [0, 1, 2, 3, 5, 8] {
            let bytes = vec![0xAB; length];

            let (type_name, fallback, raw, status) = convert_registry_value(RegType::U32, &bytes);

            assert_eq!(type_name, ValueTypeName::RegDword);

            assert_eq!(raw, bytes);

            assert_eq!(fallback, RegistryValue::Binary(bytes));

            assert!(matches!(status, DecodingStatus::Malformed { .. }));
        }
    }

    #[test]
    fn reg_qword_decodes_little_endian_values() {
        for value in [
            0u64,
            1,
            42,
            u32::MAX as u64 + 1,
            0x123456789ABCDEF0,
            u64::MAX,
        ] {
            let bytes = value.to_le_bytes();

            let (type_name, decoded, raw, status) = convert_registry_value(RegType::U64, &bytes);

            assert_eq!(type_name, ValueTypeName::RegQword);

            assert_eq!(type_name.to_raw_id(), 11);

            assert_eq!(decoded, RegistryValue::Qword(value));

            assert_eq!(raw, bytes.to_vec());

            assert_eq!(status, DecodingStatus::Decoded);
        }
    }

    #[test]
    fn reg_qword_rejects_invalid_lengths() {
        for length in [0, 1, 4, 7, 9, 16] {
            let bytes = vec![0xCD; length];

            let (type_name, fallback, raw, status) = convert_registry_value(RegType::U64, &bytes);

            assert_eq!(type_name, ValueTypeName::RegQword);

            assert_eq!(raw, bytes);

            assert_eq!(fallback, RegistryValue::Binary(bytes));

            assert!(matches!(status, DecodingStatus::Malformed { .. }));
        }
    }

    #[test]
    fn reg_binary_preserves_arbitrary_payloads() {
        let payloads = [
            Vec::new(),
            vec![0],
            vec![0, 255, 128, 1, 0, 42],
            (0u8..=255).collect::<Vec<u8>>(),
        ];

        for bytes in payloads {
            let (type_name, decoded, raw, status) = convert_registry_value(RegType::Bytes, &bytes);

            assert_eq!(type_name, ValueTypeName::RegBinary);

            assert_eq!(type_name.to_raw_id(), 3);

            assert_eq!(raw, bytes);

            assert_eq!(decoded, RegistryValue::Binary(bytes));

            assert_eq!(status, DecodingStatus::Decoded);
        }
    }

    #[test]
    fn reg_none_preserves_raw_payload() {
        for bytes in [Vec::new(), vec![0, 1, 255, 42]] {
            let (type_name, fallback, raw, status) =
                convert_registry_value(RegType::Other(0), &bytes);

            assert_eq!(type_name, ValueTypeName::Other(0));

            assert_eq!(type_name.to_raw_id(), 0);

            assert_eq!(raw, bytes);

            assert_eq!(fallback, RegistryValue::Binary(bytes));

            assert_eq!(status, DecodingStatus::UnknownType);
        }
    }

    #[test]
    fn unsupported_registry_types_preserve_numeric_ids() {
        for id in [5u32, 6, 8, 9, 10, 12, 999, u32::MAX] {
            let bytes = vec![0, 1, 128, 255];

            let (type_name, fallback, raw, status) =
                convert_registry_value(RegType::Other(id), &bytes);

            assert_eq!(type_name, ValueTypeName::Other(id));

            assert_eq!(type_name.to_raw_id(), id);

            assert_eq!(raw, bytes);

            assert_eq!(fallback, RegistryValue::Binary(bytes));

            assert_eq!(status, DecodingStatus::UnknownType);
        }
    }

    #[test]
    fn registry_type_mapping_preserves_win32_ids() {
        let cases = [
            (RegType::String, ValueTypeName::RegSz, 1u32),
            (RegType::ExpandString, ValueTypeName::RegExpandSz, 2),
            (RegType::Bytes, ValueTypeName::RegBinary, 3),
            (RegType::U32, ValueTypeName::RegDword, 4),
            (RegType::MultiString, ValueTypeName::RegMultiSz, 7),
            (RegType::U64, ValueTypeName::RegQword, 11),
            (RegType::Other(0), ValueTypeName::Other(0), 0),
            (RegType::Other(999), ValueTypeName::Other(999), 999),
        ];

        for (windows_type, expected_type, id) in cases {
            let converted = convert_type(windows_type);

            assert_eq!(converted, expected_type);

            assert_eq!(converted.to_raw_id(), id);
        }
    }

    #[test]
    fn missing_optional_root_is_not_an_error() {
        let reader = FakeReader::new(BTreeMap::new());

        let (snapshot, budget) = inspect_optional_test_tree(&reader, MAX_KEYS_PER_DOMAIN);

        assert!(snapshot.bags.is_empty());

        assert_eq!(count_snapshot_read_errors(&snapshot), 0);

        assert_eq!(budget.visited_keys, 1);

        assert!(!budget.limit_exceeded);

        assert!(matches!(snapshot.status, InspectionStatus::Complete));

        assert_eq!(reader.opened_keys.lock().unwrap().len(), 1);
    }

    #[test]
    fn optional_root_access_denied_remains_an_error() {
        let reader = FakeReader::new(BTreeMap::new()).with_denied_key("root");

        let (snapshot, budget) = inspect_optional_test_tree(&reader, MAX_KEYS_PER_DOMAIN);

        let root = snapshot.bags.get("root").expect("Error snapshot missing");

        assert_eq!(root.read_errors.len(), 1);

        assert_eq!(root.read_errors[0].operation, "open_key");

        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert_eq!(budget.visited_keys, 1);

        assert!(matches!(snapshot.status, InspectionStatus::Failed { .. }));
    }

    #[test]
    fn missing_enumerated_child_remains_an_error() {
        let mut data = BTreeMap::new();

        data.insert("root".to_string(), vec!["child".to_string()]);

        let reader = FakeReader::new(data);

        let (snapshot, budget) = inspect_optional_test_tree(&reader, MAX_KEYS_PER_DOMAIN);

        let root = snapshot.bags.get("root").expect("Root missing");

        assert!(matches!(
            root.values.get("TestValue"),
            Some(ValueReadResult::Success(_))
        ));

        let child = root
            .subkeys
            .get(r"root\child")
            .expect("Missing child error snapshot");

        assert_eq!(child.read_errors.len(), 1);

        assert_eq!(child.read_errors[0].operation, "open_key");

        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert_eq!(budget.visited_keys, 2);

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn existing_optional_root_is_inspected_normally() {
        let reader = build_hierarchy(1);

        let (snapshot, budget) = inspect_optional_test_tree(&reader, MAX_KEYS_PER_DOMAIN);

        let root = snapshot.bags.get("root").expect("Root missing");

        assert!(matches!(
            root.values.get("TestValue"),
            Some(ValueReadResult::Success(_))
        ));

        let child = root.subkeys.get(r"root\child").expect("Child missing");

        assert!(matches!(
            child.values.get("TestValue"),
            Some(ValueReadResult::Success(_))
        ));

        assert_eq!(count_snapshot_read_errors(&snapshot), 0);

        assert_eq!(budget.visited_keys, 2);

        assert!(matches!(snapshot.status, InspectionStatus::Complete));
    }

    #[test]
    fn missing_optional_roots_across_recursive_domains_are_not_errors() {
        let reader = FakeReader::new(BTreeMap::new());

        let mut snapshot = InspectionSnapshot::default();

        let mut snapshot_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

        inspect_bags(&reader, &mut snapshot, &mut snapshot_budget);

        inspect_streams(&reader, &mut snapshot, &mut snapshot_budget);

        inspect_clsid(&reader, &mut snapshot, &mut snapshot_budget);

        inspect_policies(&reader, &mut snapshot, &mut snapshot_budget);

        finalize_inspection_status(&mut snapshot);

        assert!(snapshot.bags.is_empty());
        assert!(snapshot.streams.is_empty());
        assert!(snapshot.clsid.is_empty());
        assert!(snapshot.policies.is_empty());

        assert_eq!(count_snapshot_read_errors(&snapshot), 0);

        assert_eq!(reader.opened_keys.lock().unwrap().len(), 23);

        assert!(matches!(snapshot.status, InspectionStatus::Complete));
    }

    #[test]
    fn optional_root_does_not_bypass_key_limit() {
        let reader = FakeReader::new(BTreeMap::new());

        let (snapshot, budget) = inspect_optional_test_tree(&reader, 0);

        assert!(budget.limit_exceeded);

        assert_eq!(budget.visited_keys, 0);

        assert!(reader.opened_keys.lock().unwrap().is_empty());

        let root = snapshot
            .bags
            .get("root")
            .expect("Key limit snapshot missing");

        assert_eq!(root.read_errors.len(), 1);

        assert_eq!(root.read_errors[0].operation, "key_limit");

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn folder_types_root_absence_respects_hive_policy() {
        let reader = FakeReader::new(BTreeMap::new());

        let (hkcu, budget) = inspect_specialized_folder_types(&reader, RootKey::Hkcu);

        assert!(hkcu.hkcu_folder_types.types.is_empty());
        assert!(hkcu.hkcu_folder_types.read_errors.is_empty());

        assert_eq!(count_snapshot_read_errors(&hkcu), 0);

        assert_eq!(budget.captured_bytes, 0);

        assert!(matches!(hkcu.status, InspectionStatus::Complete));

        let (hklm, budget) = inspect_specialized_folder_types(&reader, RootKey::Hklm);

        assert!(hklm.hklm_folder_types.types.is_empty());

        assert_eq!(hklm.hklm_folder_types.read_errors.len(), 1);

        let error = &hklm.hklm_folder_types.read_errors[0];

        assert_eq!(error.root, RootKey::Hklm);
        assert_eq!(error.key_path, TEST_FOLDER_TYPES);
        assert_eq!(error.operation, "open_key");
        assert_eq!(budget.captured_bytes, 0);

        assert_eq!(count_snapshot_read_errors(&hklm), 1);

        assert!(matches!(hklm.status, InspectionStatus::Failed { .. }));
    }

    #[test]
    fn folder_types_root_access_denied_remains_error() {
        let reader = FakeReader::new(BTreeMap::new()).with_denied_key(TEST_FOLDER_TYPES);

        for root in [RootKey::Hkcu, RootKey::Hklm] {
            let (snapshot, budget) = inspect_specialized_folder_types(&reader, root);

            let folder_types = match root {
                RootKey::Hkcu => &snapshot.hkcu_folder_types,
                RootKey::Hklm => &snapshot.hklm_folder_types,
            };

            assert!(folder_types.types.is_empty());

            assert_eq!(folder_types.read_errors.len(), 1);

            assert_eq!(folder_types.read_errors[0].root, root);

            assert_eq!(folder_types.read_errors[0].operation, "open_key");

            assert_eq!(count_snapshot_read_errors(&snapshot), 1);

            assert_eq!(budget.captured_bytes, 0);

            assert!(matches!(snapshot.status, InspectionStatus::Failed { .. }));
        }
    }

    #[test]
    fn advanced_root_absence_and_access_denied_are_distinct() {
        use winview_core::registry::keys::paths::ADVANCED;

        for denied in [false, true] {
            let reader = FakeReader::new(BTreeMap::new());

            let reader = if denied {
                reader.with_denied_key(ADVANCED)
            } else {
                reader
            };

            let mut snapshot = InspectionSnapshot::default();

            let mut budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

            inspect_advanced(&reader, &mut snapshot, MAX_VALUE_BYTES, &mut budget);

            finalize_inspection_status(&mut snapshot);

            assert!(snapshot.advanced.values.is_empty());
            assert!(snapshot.advanced.subkeys.is_empty());
            assert_eq!(budget.captured_bytes, 0);

            if denied {
                assert_eq!(snapshot.advanced.read_errors.len(), 1);

                assert_eq!(snapshot.advanced.read_errors[0].operation, "open_key");

                assert!(matches!(snapshot.status, InspectionStatus::Failed { .. }));
            } else {
                assert!(snapshot.advanced.read_errors.is_empty());

                assert!(matches!(snapshot.status, InspectionStatus::Complete));
            }
        }
    }

    #[test]
    fn missing_topviews_preserves_folder_type() {
        let view_path = format!(
            r"{}\{}\TopViews\{}",
            TEST_FOLDER_TYPES, TEST_FOLDER_GUID, TEST_VIEW_GUID,
        );

        for root in [RootKey::Hkcu, RootKey::Hklm] {
            for canonical_name in [Some("A"), None] {
                let mut reader = build_folder_types_fixture(canonical_name, false);

                Arc::make_mut(&mut reader.data).remove(&view_path);

                let (snapshot, budget) = inspect_specialized_folder_types(&reader, root);

                let folder_types = match root {
                    RootKey::Hkcu => &snapshot.hkcu_folder_types,
                    RootKey::Hklm => &snapshot.hklm_folder_types,
                };

                let folder = folder_types
                    .types
                    .get(TEST_FOLDER_GUID)
                    .expect("FolderType discarded");

                assert_eq!(folder.guid, TEST_FOLDER_GUID);

                assert_eq!(folder.canonical_name.as_deref(), canonical_name);

                assert!(folder.top_views.is_empty());
                assert!(folder.read_errors.is_empty());

                assert_eq!(count_snapshot_read_errors(&snapshot), 0);

                let expected_bytes = if canonical_name.is_some() { 4 } else { 0 };

                assert_eq!(budget.captured_bytes, expected_bytes);

                assert_eq!(
                    snapshot_has_successful_observations(&snapshot),
                    canonical_name.is_some()
                );

                assert!(matches!(snapshot.status, InspectionStatus::Complete));
            }
        }
    }

    #[test]
    fn denied_topviews_preserves_canonical_name() {
        let view_path = format!(
            r"{}\{}\TopViews\{}",
            TEST_FOLDER_TYPES, TEST_FOLDER_GUID, TEST_VIEW_GUID,
        );

        let reader = build_folder_types_fixture(Some("A"), false).with_denied_key(&view_path);

        let (snapshot, budget) = inspect_specialized_folder_types(&reader, RootKey::Hkcu);

        let folder = snapshot
            .hkcu_folder_types
            .types
            .get(TEST_FOLDER_GUID)
            .expect("FolderType discarded");

        assert_eq!(folder.canonical_name.as_deref(), Some("A"));

        assert!(folder.top_views.is_empty());

        assert_eq!(budget.captured_bytes, 4);

        assert_eq!(snapshot.hkcu_folder_types.read_errors.len(), 1);

        let error = &snapshot.hkcu_folder_types.read_errors[0];

        assert_eq!(error.key_path, view_path);
        assert_eq!(error.operation, "open_key");

        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert!(snapshot_has_successful_observations(&snapshot));

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn denied_topviews_without_observations_is_failed() {
        let view_path = format!(
            r"{}\{}\TopViews\{}",
            TEST_FOLDER_TYPES, TEST_FOLDER_GUID, TEST_VIEW_GUID,
        );

        let reader = build_folder_types_fixture(None, false).with_denied_key(&view_path);

        let (snapshot, budget) = inspect_specialized_folder_types(&reader, RootKey::Hkcu);

        let folder = snapshot
            .hkcu_folder_types
            .types
            .get(TEST_FOLDER_GUID)
            .expect("FolderType discarded");

        assert!(folder.canonical_name.is_none());
        assert!(folder.top_views.is_empty());

        assert_eq!(budget.captured_bytes, 0);

        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert!(!snapshot_has_successful_observations(&snapshot));

        assert!(matches!(snapshot.status, InspectionStatus::Failed { .. }));
    }

    #[test]
    fn missing_enumerated_folder_guid_remains_error() {
        let missing_guid = "{22222222-2222-2222-2222-222222222222}";

        let missing_path = format!(r"{}\{}", TEST_FOLDER_TYPES, missing_guid,);

        let mut reader = build_folder_types_fixture(Some("A"), false);

        Arc::make_mut(&mut reader.data)
            .get_mut(TEST_FOLDER_TYPES)
            .unwrap()
            .insert(0, missing_guid.to_string());

        let (snapshot, budget) = inspect_specialized_folder_types(&reader, RootKey::Hkcu);

        assert!(!snapshot.hkcu_folder_types.types.contains_key(missing_guid));

        let folder = snapshot
            .hkcu_folder_types
            .types
            .get(TEST_FOLDER_GUID)
            .expect("Later folder was not inspected");

        assert_eq!(folder.canonical_name.as_deref(), Some("A"));

        let view = folder.top_views.get(TEST_VIEW_GUID).expect("View missing");

        assert_eq!(view.mode, Some(42));

        assert_eq!(budget.captured_bytes, 8);

        assert_eq!(snapshot.hkcu_folder_types.read_errors.len(), 1);

        let error = &snapshot.hkcu_folder_types.read_errors[0];

        assert_eq!(error.key_path, missing_path);
        assert_eq!(error.operation, "open_key");

        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert!(matches!(snapshot.status, InspectionStatus::Partial { .. }));
    }

    #[test]
    fn folder_types_zero_key_budget_prevents_opening() {
        let reader = build_folder_types_fixture(Some("A"), false);

        let (snapshot, bytes, traversal) = inspect_folder_types_with_key_limit(&reader, 0);

        assert!(reader.opened_keys.lock().unwrap().is_empty());

        assert_eq!(traversal.visited_keys, 0);
        assert!(traversal.limit_exceeded);
        assert_eq!(bytes.captured_bytes, 0);

        assert!(snapshot.hkcu_folder_types.types.is_empty());

        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert_folder_types_key_limit(&snapshot);
    }

    #[test]
    fn folder_types_exact_key_capacity_is_complete() {
        let reader = build_folder_types_fixture(Some("A"), false);

        let (snapshot, bytes, traversal) = inspect_folder_types_with_key_limit(&reader, 3);

        assert_eq!(traversal.visited_keys, 3);
        assert!(!traversal.limit_exceeded);

        assert_eq!(reader.opened_keys.lock().unwrap().len(), 3);

        let folder = snapshot
            .hkcu_folder_types
            .types
            .get(TEST_FOLDER_GUID)
            .unwrap();

        assert_eq!(folder.canonical_name.as_deref(), Some("A"));

        assert_eq!(folder.top_views[TEST_VIEW_GUID].mode, Some(42));

        assert_eq!(bytes.captured_bytes, 8);

        assert_eq!(count_snapshot_read_errors(&snapshot), 0);

        assert!(matches!(snapshot.status, InspectionStatus::Complete));
    }

    #[test]
    fn folder_types_limit_before_guid_open() {
        let reader = build_folder_types_fixture(Some("A"), false);

        let (snapshot, bytes, traversal) = inspect_folder_types_with_key_limit(&reader, 1);

        assert_eq!(traversal.visited_keys, 1);

        assert_eq!(reader.opened_keys.lock().unwrap().len(), 1);

        assert!(traversal.limit_exceeded);
        assert_eq!(bytes.captured_bytes, 0);

        assert!(snapshot.hkcu_folder_types.types.is_empty());

        let error = &snapshot.hkcu_folder_types.read_errors[0];

        assert_eq!(
            error.key_path,
            format!(r"{}\{}", TEST_FOLDER_TYPES, TEST_FOLDER_GUID,)
        );

        assert_folder_types_key_limit(&snapshot);
    }

    #[test]
    fn folder_types_limit_preserves_canonical_name() {
        let reader = build_folder_types_fixture(Some("A"), false);

        let (snapshot, bytes, traversal) = inspect_folder_types_with_key_limit(&reader, 2);

        assert_eq!(traversal.visited_keys, 2);

        assert_eq!(reader.opened_keys.lock().unwrap().len(), 2);

        assert!(traversal.limit_exceeded);

        let folder = snapshot
            .hkcu_folder_types
            .types
            .get(TEST_FOLDER_GUID)
            .expect("Captured FolderType was lost");

        assert_eq!(folder.canonical_name.as_deref(), Some("A"));

        assert!(folder.top_views.is_empty());

        assert_eq!(bytes.captured_bytes, 4);

        let error = &snapshot.hkcu_folder_types.read_errors[0];

        assert_eq!(
            error.key_path,
            format!(
                r"{}\{}\TopViews\{}",
                TEST_FOLDER_TYPES, TEST_FOLDER_GUID, TEST_VIEW_GUID,
            )
        );

        assert_folder_types_key_limit(&snapshot);
    }

    #[test]
    fn folder_types_failed_openings_consume_budget() {
        let missing_guid = "{22222222-2222-2222-2222-222222222222}";

        let mut reader = build_folder_types_fixture(Some("A"), false);

        Arc::make_mut(&mut reader.data)
            .get_mut(TEST_FOLDER_TYPES)
            .unwrap()
            .insert(0, missing_guid.to_string());

        let (snapshot, bytes, traversal) = inspect_folder_types_with_key_limit(&reader, 3);

        assert_eq!(traversal.visited_keys, 3);
        assert!(traversal.limit_exceeded);

        let opened = reader.opened_keys.lock().unwrap();

        assert_eq!(opened.len(), 3);

        assert_eq!(
            opened[1].1,
            format!(r"{}\{}", TEST_FOLDER_TYPES, missing_guid,)
        );

        assert_eq!(snapshot.hkcu_folder_types.read_errors.len(), 2);

        assert_eq!(
            snapshot.hkcu_folder_types.read_errors[0].operation,
            "open_key"
        );

        let folder = snapshot
            .hkcu_folder_types
            .types
            .get(TEST_FOLDER_GUID)
            .unwrap();

        assert_eq!(folder.canonical_name.as_deref(), Some("A"));

        assert!(folder.top_views.is_empty());

        assert_eq!(bytes.captured_bytes, 4);

        assert_folder_types_key_limit(&snapshot);
    }

    #[test]
    fn folder_types_key_budget_is_shared_across_hives() {
        use winview_core::registry::keys::paths::{HKCU_FOLDER_TYPES, HKLM_FOLDER_TYPES};

        // Both hive paths are identical relative
        // to their respective registry roots.
        assert_eq!(HKCU_FOLDER_TYPES, HKLM_FOLDER_TYPES);

        let root_path = HKCU_FOLDER_TYPES;

        let guid_path = format!(r"{}\{}", root_path, TEST_FOLDER_GUID,);

        let view_path = format!(r"{}\TopViews\{}", guid_path, TEST_VIEW_GUID,);

        let mut data = BTreeMap::new();

        data.insert(root_path.to_string(), vec![TEST_FOLDER_GUID.to_string()]);

        data.insert(guid_path.clone(), Vec::new());

        data.insert(view_path.clone(), Vec::new());

        let mut values = FakeValues::new();

        values.entry(guid_path).or_default().insert(
            "CanonicalName".into(),
            (
                1,
                utf16_units_to_bytes(&[65, 0]),
                Some(RegistryValue::String("A".into())),
                DecodingStatus::Decoded,
            ),
        );

        values.entry(view_path).or_default().insert(
            "Mode".into(),
            (
                4,
                42u32.to_le_bytes().to_vec(),
                Some(RegistryValue::Dword(42)),
                DecodingStatus::Decoded,
            ),
        );

        for max_keys in [2, 3, 6] {
            let reader = FakeReader::with_values(data.clone(), values.clone());

            let mut snapshot = InspectionSnapshot::default();

            let mut byte_budget = SnapshotBudget::new(MAX_SNAPSHOT_BYTES);

            inspect_folder_types(&reader, &mut snapshot, &mut byte_budget, max_keys);

            finalize_inspection_status(&mut snapshot);

            let opened = reader.opened_keys.lock().unwrap();

            assert_eq!(opened.len(), max_keys);

            let hkcu_folder = &snapshot.hkcu_folder_types.types[TEST_FOLDER_GUID];

            assert_eq!(hkcu_folder.canonical_name.as_deref(), Some("A"));

            if max_keys == 2 {
                assert!(hkcu_folder.top_views.is_empty());

                assert!(snapshot.hklm_folder_types.read_errors.is_empty());

                assert_folder_types_key_limit(&snapshot);
            } else if max_keys == 3 {
                assert_eq!(hkcu_folder.top_views[TEST_VIEW_GUID].mode, Some(42));

                assert_eq!(snapshot.hklm_folder_types.read_errors.len(), 1);

                assert_eq!(
                    snapshot.hklm_folder_types.read_errors[0].root,
                    RootKey::Hklm
                );

                assert_folder_types_key_limit(&snapshot);
            } else {
                assert_eq!(
                    snapshot.hklm_folder_types.types[TEST_FOLDER_GUID].top_views[TEST_VIEW_GUID]
                        .mode,
                    Some(42)
                );

                assert_eq!(count_snapshot_read_errors(&snapshot), 0);

                assert_eq!(byte_budget.captured_bytes, 16);

                assert!(matches!(snapshot.status, InspectionStatus::Complete));
            }

            if max_keys < 6 {
                assert!(snapshot.hklm_folder_types.types.is_empty());

                assert!(opened.iter().all(|(root, _)| { *root == RootKey::Hkcu }));
            }
        }
    }

    #[test]
    fn native_enumeration_collects_entries_until_end() {
        let mut visited = Vec::new();

        let entries = collect_registry_entries(|index| {
            visited.push(index);

            match index {
                0 => Ok(Some("First")),
                1 => Ok(Some("Second")),
                2 => Ok(None),
                _ => panic!("Enumeration continued after termination"),
            }
        })
        .unwrap();

        assert_eq!(entries, vec!["First", "Second"]);

        assert_eq!(visited, vec![0, 1, 2]);

        let empty = collect_registry_entries(|_| Ok(None::<String>)).unwrap();

        assert!(empty.is_empty());
    }

    #[test]
    fn native_enumeration_propagates_midstream_errors() {
        let mut visited = Vec::new();

        let result = collect_registry_entries(|index| {
            visited.push(index);

            match index {
                0 => Ok(Some("First")),
                1 => Err(RegistryError::WindowsApi(
                    "injected enumeration failure".into(),
                )),
                _ => panic!("Enumeration continued after failure"),
            }
        });

        assert!(matches!(
            result,
            Err(RegistryError::WindowsApi(message))
                if message == "injected enumeration failure"
        ));

        assert_eq!(visited, vec![0, 1]);
    }

    #[test]
    fn native_enumeration_distinguishes_end_from_error() {
        let buffer = [65u16, 0];

        let ended = decode_enumerated_name(
            ERROR_NO_MORE_ITEMS,
            &buffer,
            u32::MAX,
            RootKey::Hkcu,
            "Software\\Test",
            "RegEnumValueW",
            7,
        )
        .unwrap();

        assert!(ended.is_none());

        for code in [
            2u32, // FILE_NOT_FOUND
            5,    // ACCESS_DENIED
            6,    // INVALID_HANDLE
            234,  // MORE_DATA
            1018, // KEY_DELETED
        ] {
            let error = decode_enumerated_name(
                WIN32_ERROR(code),
                &buffer,
                1,
                RootKey::Hkcu,
                "Software\\Test",
                "RegEnumValueW",
                7,
            )
            .unwrap_err();

            match error {
                RegistryError::WindowsApi(message) => {
                    assert!(message.contains("RegEnumValueW"));

                    assert!(message.contains("Software\\Test"));

                    assert!(message.contains("Hkcu"));

                    assert!(message.contains("index 7"));

                    assert!(message.contains(&format!("Win32 error {code}")));
                }
                other => panic!("Unexpected error: {other:?}"),
            }
        }
    }

    #[test]
    fn native_enumeration_preserves_unicode_and_default_names() {
        for expected in ["", "Mode", "안녕하세요", "Explorer 😀"] {
            let units: Vec<u16> = expected.encode_utf16().chain(std::iter::once(0)).collect();

            let result = decode_enumerated_name(
                ERROR_SUCCESS,
                &units,
                (units.len() - 1) as u32,
                RootKey::Hkcu,
                "Software\\Test",
                "RegEnumValueW",
                0,
            )
            .unwrap();

            assert_eq!(result.as_deref(), Some(expected));
        }
    }

    #[test]
    fn native_enumeration_rejects_invalid_names() {
        let cases = [
            (vec![65u16, 0], 2u32),
            (vec![65, 0], u32::MAX),
            (vec![0xD800, 0], 1),
            (vec![0xDC00, 0], 1),
            (vec![65, 0, 66, 0], 3),
        ];

        for (buffer, length) in cases {
            let result = decode_enumerated_name(
                ERROR_SUCCESS,
                &buffer,
                length,
                RootKey::Hkcu,
                "Software\\Test",
                "RegEnumValueW",
                0,
            );

            assert!(matches!(result, Err(RegistryError::InvalidData(_))));
        }
    }

    #[test]
    fn native_enumeration_preserves_raw_type_ids() {
        for raw_type in [0u32, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 999, u32::MAX] {
            let converted = convert_type(RegType::from(raw_type));

            assert_eq!(converted.to_raw_id(), raw_type);
        }
    }
}
