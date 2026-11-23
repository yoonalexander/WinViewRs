// winview-core/src/model.rs

use crate::registry::keys::RootKey;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Decoded registry value — platform-neutral representation
/// Unknown/unsupported registry types are NOT represented here; use RawRegistryValue for that
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "data")]
pub enum RegistryValue {
    String(String),
    ExpandString(String),
    Dword(u32),
    Qword(u64),
    Binary(Vec<u8>),
    MultiString(Vec<String>),
    // REG_NONE and other unknown types are NOT represented here.
    // They appear only in RawRegistryValue with raw_bytes and type_id.
}

/// Status of decoding a registry value from raw bytes
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", content = "detail")]
pub enum DecodingStatus {
    /// Successfully decoded to a RegistryValue
    Decoded,
    /// Recognized type but malformed data (wrong length, invalid UTF-16, etc.)
    /// raw_bytes preserved; decoded = None
    Malformed { reason: String },
    /// Unrecognized Windows type ID (Other(u32))
    /// raw_bytes preserved; decoded = None
    UnknownType,
}

/// Error that can occur when reading a registry value
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "detail")]
pub enum ReadError {
    KeyNotFound { path: String },
    ValueNotFound { name: String },
    AccessDenied { path: String },
    Other(String),
}

/// Successful raw registry observation — ALL metadata guaranteed present
/// Raw bytes ALWAYS preserved regardless of decode success
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawRegistryValue {
    pub key_path: String,   // Full path from root (e.g., "Software\\Microsoft\\...")
    pub value_name: String, // Empty string = default value
    pub root: RootKey,      // Which hive this came from
    pub raw_type_id: u32, // Windows REG_* numeric type ID (e.g., 1=REG_SZ, 4=REG_DWORD, 0=REG_NONE)
    pub raw_bytes: Vec<u8>, // ALWAYS present — authoritative source
    pub decoded: Option<RegistryValue>, // Present only when DecodingStatus::Decoded
    pub decoding_status: DecodingStatus,
    pub source: ObservationSource, // Which inspection domain this came from
}

/// Result of attempting to read a registry value — distinguishes success from failure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ValueReadResult {
    Success(RawRegistryValue),
    Failure(ReadError),
}

/// Source of the observation (distinguishes HKLM vs HKCU even for same domain)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ObservationSource {
    HkcuFolderTypes,
    HklmFolderTypes,
    HkcuBags,
    HkcuStreams,
    HkcuAdvanced,
    HkcuClsid,
    HkcuPolicies,
    HklmPolicies,
}

/// Overall inspection completion status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "state", content = "detail")]
pub enum InspectionStatus {
    #[default]
    Complete,
    Partial {
        reason: String,
    },
    Failed {
        error: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SnapshotMetadata {
    pub captured_at: String,        // ISO 8601 with timezone
    pub windows_build: Option<u32>, // None if detection failed
    pub windows_ubr: Option<u32>,   // None if unavailable
    pub windows_product_name: Option<String>,
    pub detection_method: DetectionMethod,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DetectionMethod {
    #[default]
    Failed,
    RtlGetVersion,
    RegistryFallback,
}

/// Complete read-only snapshot of all inspected registry state
/// HKLM and HKCU observations are kept separate at the top level
/// Contains raw observations + any read errors — NOT claimed suitable for lossless restoration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InspectionSnapshot {
    pub metadata: SnapshotMetadata,
    pub status: InspectionStatus,
    // FolderTypes kept separate by hive
    pub hkcu_folder_types: FolderTypesSnapshot,
    pub hklm_folder_types: FolderTypesSnapshot,
    pub bags: BTreeMap<String, KeySnapshot>, // key = subkey path
    pub streams: BTreeMap<String, KeySnapshot>,
    pub advanced: KeySnapshot,
    pub clsid: BTreeMap<String, KeySnapshot>,
    pub policies: BTreeMap<String, KeySnapshot>,
    pub errors: Vec<InspectionError>, // Aggregated read errors (key-open failures, etc.)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FolderTypesSnapshot {
    // Key = folder type GUID
    pub types: BTreeMap<String, FolderTypeSnapshot>,
    pub read_errors: Vec<InspectionError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectionError {
    pub key_path: String,
    pub root: RootKey,
    pub operation: String, // e.g., "open_key", "enum_values", "get_value"
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FolderTypeSnapshot {
    pub guid: String,
    pub canonical_name: Option<String>,
    pub top_views: BTreeMap<String, ViewSnapshot>, // key = view GUID
    pub read_errors: Vec<InspectionError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ViewSnapshot {
    pub logical_view_mode: Option<u32>,
    pub mode: Option<u32>,
    pub icon_size: Option<u32>,
    pub group_by: Option<String>,
    pub group_ascending: Option<u32>,
    pub sort_by_list: Option<String>,
    pub column_list: Option<String>,
    pub col_info: Option<Vec<u8>>,
    pub f_flags: Option<u32>,
    pub raw_values: BTreeMap<String, ValueReadResult>, // All raw values from this view key
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KeySnapshot {
    pub values: BTreeMap<String, ValueReadResult>, // Values with type/error info
    pub subkeys: BTreeMap<String, KeySnapshot>,
    pub read_errors: Vec<InspectionError>,
}
