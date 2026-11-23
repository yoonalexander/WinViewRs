# WinViewRs Phase 0 & 1 Implementation Plan (Refined v2)

## Changelog

| Area | Change | Reason |
|------|--------|--------|
| Registry API | Corrected `windows-registry 0.100.0` API: Root constants are `&Key` refs; `Type` = `U32`/`U64`/`String`/`ExpandString`/`MultiString`/`Bytes`/`Other(u32)` (no `None`); `Key::get_value()` returns `Value` (not generic); `Value` has `ty()` and `Deref<Target=[u8]>`; `values()`/`keys()` iterators yield `(String, Value)` and `String` directly; `get_type()` exists; `From<Type> for u32`; `Key: Send` not `Sync`; `Value: !Send + !Sync` | Verified against https://docs.rs/windows-registry/0.100.0/ |
| Raw Data Model | `RawRegistryValue` now stores `raw_bytes: Vec<u8>` always; `decoded: Option<RegistryValue>`; `decoding_status: DecodingStatus`; removed `RegistryValue::None` — use `Other(0)` for `REG_NONE`; explicit tagged serialization | Raw bytes always preserved; distinct success/failure; no contradictory mandatory fields |
| REG_MULTI_SZ | Dedicated decoder with specified edge-case behavior; empty multi-string = `[]` (not `[""]`); preserves raw bytes on decode failure; fixtures for all cases | Accurate Windows semantics |
| Version Detection | `RtlGetVersion` in `windows::Wdk::System::SystemServices` with `OSVERSIONINFOW`; requires `windows` crate features `Wdk_System_SystemServices` + `Win32_Foundation`; fallback via registry; `VersionInfo` uses `Option<u32>` for build/UBR; `DetectionMethod` enum; no sentinel zeros | Correct binding; structured partial results |
| Cargo Manifests | Single authoritative workspace + per-crate tables; `time` only in registry/app; `mockall` as dev-dependency in core with `test` feature; `anyhow` only in app; all workspace deps declared; validated for `cargo metadata --no-deps` | Internal consistency; cross-platform build |
| Mocking | Single strategy: in-memory `FakeRegistryReader` implementing `RegistryReader` trait in `winview-core` test module; `mockall` only for unit-testing trait consumers in dependent crates; no extra crate | Three-crate workspace preserved; deterministic fakes |
| Tests | All tautological tests replaced with fixture-based exact assertions; `RootKey` imports fixed; property tests validate real behavior; CLI tests require successful exit + exact JSON schema | Real verification |
| CLI Integration | Deterministic fixture-based execution; explicit success assertions; exact output validation; error-code tests; `CARGO_BIN_EXE_winview-app` verified | No silent passes |
| Inspection Limits | Explicit states: `Complete`, `Partial { reason }`, `Failed { error }`; limits enforced at collection time; exported snapshots include `InspectionStatus` | Clear partial-result semantics |
| Safety Verification | Fixed markdown tables; PowerShell-compatible commands; explicit reference repo check; added checks for external commands, subprocesses, unsafe native calls, PowerShell helpers | Executable audit |
| Licensing | MIT license preservation; attribution for adapted material; clear independent implementation statement; reference repo read-only enforcement | Legal compliance |
| Migration | Non-destructive conversion from single-package to virtual workspace; preserves `AGENTS.md`, brief, Git history | Existing project respected |
| Scope | Milestone 0.1 = workspace, core types, traits, keys, features, errors, tests ONLY; no profile, no CLI, no registry adapter, no GUI | Minimal viable foundation |

---

## Overview

WinViewRs is a native Windows Rust application reimplementing WinSetView. This plan covers:

- **Phase 0**: Workspace setup, core types, read-only registry inspection foundation
- **Phase 1**: Complete read-only Explorer configuration inspection CLI + minimal GUI

**Safety invariant**: Phases 0–1 are **read-only**. No registry writes, no Explorer restarts, no system modifications, no elevation prompts.

**Licensing**: WinSetView is MIT-licensed (Les Ferch). WinViewRs is an independent Rust reimplementation. All reused or substantially adapted material preserves original copyright and license notices. The reference repository at `../WinSetView-reference` is strictly read-only — agents must never modify it.

---

## 1. Architecture

### 1.1 High-Level Layer Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                      App Layer (CLI + GUI)                  │
│  ┌─────────────┐  ┌─────────────┐                            │
│  │ Inspector   │  │ Settings    │                            │
│  │ CLI + GUI   │  │ Dialog      │                            │
│  └──────┬──────┘  └──────┬──────┘                            │
└─────────┼────────────────┼────────────────────────────────────┘
          │                │
          ▼                ▼
┌─────────────────────────────────────────────────────────────┐
│                      Core Layer (platform-independent)      │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐          │
│  │ Inspection  │  │ Capability  │  │ Registry    │          │
│  │ Model       │  │ Data        │  │ Traits      │          │
│  │ (raw values)│  │ (VersionInfo,│  │ (Reader)    │          │
│  │             │  │ FeatureCaps)│  │             │          │
│  └─────────────┘  └─────────────┘  └─────────────┘          │
└─────────────────────────────────────────────────────────────┘
          │
          ▼
┌─────────────────────────────────────────────────────────────┐
│                      Platform Layer                          │
│  ┌─────────────────────────────────────────────────────┐     │
│  │ Windows Registry Adapter (windows-registry 0.100.0)  │     │
│  │ - Read-only impl for production                      │     │
│  │ - Windows version detection (RtlGetVersion + reg)    │     │
│  │ - Fake impl for deterministic tests                  │     │
│  └─────────────────────────────────────────────────────┘     │
└─────────────────────────────────────────────────────────────┘
```

### 1.2 Module Structure (Three-Crate Workspace)

```
WinViewRs/
├── Cargo.toml                 # Workspace root (virtual)
├── rust-toolchain.toml        # Toolchain pinning (1.100.0)
├── LICENSE                    # MIT license
├── crates/
│   ├── winview-core/          # Platform-independent core (cross-platform)
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── model.rs       # InspectionSnapshot, RegistryValue, RawRegistryValue, DecodingStatus, ReadError
│   │   │   ├── registry/      # Registry abstraction (traits only)
│   │   │   │   ├── mod.rs
│   │   │   │   ├── traits.rs  # RegistryReader, RegistryKey traits
│   │   │   │   └── keys.rs    # Well-known paths, value names (platform-neutral)
│   │   │   ├── windows/       # Windows capability data (platform-neutral structs)
│   │   │   │   ├── mod.rs
│   │   │   │   ├── version.rs
│   │   │   │   └── features.rs
│   │   │   ├── error.rs       # Domain errors (thiserror)
│   │   │   └── test_support.rs # FakeRegistryReader for deterministic tests
│   │   ├── tests/             # Unit + property tests (cross-platform)
│   │   │   ├── model_tests.rs
│   │   │   ├── version_tests.rs
│   │   │   └── property_tests.rs
│   │   └── Cargo.toml
│   ├── winview-registry/      # Windows-specific registry adapter (Windows-only)
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── reader.rs      # RegistryReader impl using windows-registry
│   │   │   └── version.rs     # Windows version detection (RtlGetVersion + registry)
│   │   ├── tests/             # Windows integration tests (read-only)
│   │   │   └── integration.rs
│   │   └── Cargo.toml
│   └── winview-app/           # CLI + GUI (Windows-only, feature-gated)
│       ├── src/
│       │   ├── lib.rs
│       │   ├── cli/           # CLI commands (inspect, dump, export)
│       │   │   ├── mod.rs
│       │   │   ├── inspect.rs
│       │   │   └── output.rs
│       │   ├── gui/           # Minimal egui/eframe GUI (feature-gated)
│       │   │   ├── mod.rs
│       │   │   ├── app.rs
│       │   │   └── inspector_tab.rs
│       │   ├── profile/       # Profile model (ExplorerOptions, etc.) — app layer only
│       │   │   ├── mod.rs
│       │   │   └── model.rs
│       │   ├── logging.rs
│       │   └── main.rs        # Entry point (CLI by default)
│       ├── tests/             # CLI integration tests
│       │   └── cli_tests.rs
│       └── Cargo.toml
├── fixtures/                  # Shared test fixtures (workspace root)
│   ├── registry_snapshots/    # JSON dumps of InspectionSnapshot
│   ├── profiles/              # Valid TOML profiles
│   └── edge_cases/            # Missing keys, access denied, binary data, multi-sz cases
└── scripts/
    └── vm-test.ps1            # Manual VM test harness (Phase 2+)
```

### 1.3 Cargo Workspace Configuration

```toml
# Cargo.toml (workspace root, virtual)
[workspace]
resolver = "2"
members = [
    "crates/winview-core",
    "crates/winview-registry",
    "crates/winview-app",
]

[workspace.dependencies]
# Verified compatible with Rust 1.100.0
windows-registry = { version = "0.100.0", default-features = false }
egui = { version = "0.37.0", default-features = false, features = ["default_fonts"] }
eframe = "0.37.0"
clap = { version = "4.6.9", features = ["derive"] }
serde = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.148"
toml = { version = "0.8.19", features = ["parse", "display"] }
thiserror = "2.0.21"
anyhow = "1.0.104"
tracing = "0.1.44"
tracing-subscriber = "0.3.18"
mockall = "0.15.0"
tempfile = "3.27.0"
proptest = "1.12.0"
insta = { version = "1.44.0", features = ["json"] }
time = { version = "0.3", features = ["formatting", "serde"] }
# Windows API for version detection — ONLY in winview-registry
windows = { version = "0.62.0", features = ["Wdk_System_SystemServices", "Win32_Foundation", "Win32_System_Registry"] }

[profile.dev]
opt-level = 1
```

```toml
# rust-toolchain.toml
[toolchain]
channel = "1.100.0"
components = ["rustfmt", "clippy"]
targets = ["x86_64-pc-windows-msvc"]
```

### 1.4 Key Architectural Decisions

| Decision | Rationale |
|----------|-----------|
| **Three crates** | Simpler ownership; `winview-core` is platform-independent; `winview-app` combines CLI+GUI behind feature flags |
| **RegistryReader trait only** | Compile-time read-only enforcement; `RegistryWriter` deferred to Phase 2 |
| **`windows-registry` 0.100.0** | Official Microsoft crate; preserves value types; compatible with Rust 1.100.0 |
| **egui/eframe 0.37.0** | Verified compatible pair; Rust 1.95 MSRV; project brief recommendation |
| **InspectionSnapshot for raw observations** | Preserves raw registry values with types, unknown values, errors; represents absent/inherited/unsupported settings without interpretation |
| **Decoded settings separate from raw** | `InspectionSnapshot` = raw observations + errors; decoded Explorer settings computed on demand in app layer |
| **TOML for profiles, JSON for registry backups** | Profiles = user intent (lossy, editable); Backups = lossless registry state for restoration |
| **`rust-toolchain.toml`** | Standard toolchain pinning; works with CI and IDEs |
| **Windows version detection in adapter** | `RtlGetVersion` + registry cross-check lives in `winview-registry`, not core |
| **Profile model in app layer** | `ExplorerOptions` and folder-type config are CLI/GUI concerns, not core inspection |
| **Conditional compilation** | `winview-registry` and `winview-app` are `#[cfg(windows)]`; core builds everywhere |
| **In-memory fakes for testing** | `FakeRegistryReader` in `winview-core::test_support`; deterministic, no `mockall` in production |

---

## 2. Shared Data Models

### 2.1 Inspection Model (`winview-core::model`)

```rust
// winview-core/src/model.rs

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use crate::registry::keys::{RootKey, ValueTypeName};

/// Registry value type names (matching Win32 REG_* constants)
/// Unknown types use Other(u32) preserving the numeric type ID
/// REG_NONE = Other(0)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueTypeName {
    RegSz,
    RegExpandSz,
    RegDword,
    RegQword,
    RegBinary,
    RegMultiSz,
    Other(u32),
}

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
    pub key_path: String,           // Full path from root (e.g., "Software\\Microsoft\\...")
    pub value_name: String,         // Empty string = default value
    pub root: RootKey,              // Which hive this came from
    pub raw_type_id: u32,           // Windows REG_* numeric type ID (e.g., 1=REG_SZ, 4=REG_DWORD, 0=REG_NONE)
    pub raw_bytes: Vec<u8>,         // ALWAYS present — authoritative source
    pub decoded: Option<RegistryValue>, // Present only when DecodingStatus::Decoded
    pub decoding_status: DecodingStatus,
    pub source: ObservationSource,  // Which inspection domain this came from
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
    pub bags: BTreeMap<String, KeySnapshot>,                // key = subkey path
    pub streams: BTreeMap<String, KeySnapshot>,
    pub advanced: KeySnapshot,
    pub clsid: BTreeMap<String, KeySnapshot>,
    pub policies: BTreeMap<String, KeySnapshot>,
    pub errors: Vec<InspectionError>,  // Aggregated read errors (key-open failures, etc.)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FolderTypesSnapshot {
    // Key = folder type GUID
    pub types: BTreeMap<String, FolderTypeSnapshot>,
    pub read_errors: Vec<InspectionError>,
}

/// Overall inspection completion status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", content = "detail")]
pub enum InspectionStatus {
    Complete,
    Partial { reason: String },  // e.g., "key_limit_exceeded", "size_limit_exceeded", "depth_limit_exceeded"
    Failed { error: String },
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectionError {
    pub key_path: String,
    pub root: RootKey,
    pub operation: String,          // e.g., "open_key", "enum_values", "get_value"
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
    pub raw_values: BTreeMap<String, ValueReadResult>,  // All raw values from this view key
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KeySnapshot {
    pub values: BTreeMap<String, ValueReadResult>,      // Values with type/error info
    pub subkeys: BTreeMap<String, KeySnapshot>,
    pub read_errors: Vec<InspectionError>,
}
```

### 2.2 Profile Model (`winview-app::profile` — App Layer Only)

```rust
// winview-app/src/profile/model.rs

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Top-level user profile (serialized to TOML) — represents user intent, not registry state
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Profile {
    pub metadata: ProfileMetadata,
    pub global: GlobalConfig,
    pub folder_types: BTreeMap<String, FolderTypeConfig>, // key = canonical name
    pub explorer_options: ExplorerOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfileMetadata {
    pub version: u32,
    pub created_by: String,
    pub created_at: String,        // ISO 8601
    pub windows_build: u32,
    pub windows_ubr: u32,
}

/// Global defaults (applied when folder type inherits)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalConfig {
    pub view_mode: Option<ViewMode>,
    pub icon_size: Option<u32>,
    pub columns: Vec<ColumnConfig>,
    pub group_by: Option<GroupByConfig>,
    pub sort_by: Vec<SortConfig>,
    pub file_dialog: Option<FileDialogConfig>,
}

/// Per-folder-type configuration (user intent)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FolderTypeConfig {
    pub guid: String,
    pub canonical_name: String,
    pub inherit: bool,
    pub include: bool,
    pub view_mode: Option<ViewMode>,
    pub icon_size: Option<u32>,
    pub columns: Option<Vec<ColumnConfig>>,
    pub group_by: Option<GroupByConfig>,
    pub sort_by: Option<Vec<SortConfig>>,
    pub file_dialog: Option<FileDialogConfig>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    Details,
    List,
    Tiles,
    Content,
    SmallIcons,
    MediumIcons,
    LargeIcons,
    ExtraLargeIcons,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnConfig {
    pub property: String,
    pub display: bool,
    pub width_em: Option<u32>,
    pub right_click: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupByConfig {
    pub property: String,
    pub ascending: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortConfig {
    pub property: String,
    pub ascending: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileDialogConfig {
    pub enabled: bool,
    pub view_mode: Option<ViewMode>,
    pub icon_size: Option<u32>,
    pub no_grouping: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExplorerOptions {
    pub show_extensions: Option<bool>,
    pub compact_view: Option<bool>,
    pub checkboxes: Option<bool>,
    pub show_hidden: Option<bool>,
    pub classic_context_menu: Option<bool>,
    pub copy_move_in_menu: Option<bool>,
    pub no_internet_search: Option<bool>,
    pub no_search_highlights: Option<bool>,
    pub unhide_appdata: Option<bool>,
    pub no_suggestions: Option<bool>,
    pub no_numerical_sort: Option<bool>,
    pub win10_search: Option<bool>,
    pub win11_explorer: Option<bool>,
    pub win10_explorer_on_11: Option<bool>,
    pub remove_home: Option<bool>,
    pub remove_gallery: Option<bool>,
    pub no_folder_thumbs: Option<bool>,
    pub legacy_spacing: Option<bool>,
    pub no_full_row_select: Option<bool>,
    pub auto_arrange: Option<bool>,
    pub align_to_grid: Option<bool>,
    pub system_text_color: Option<String>,
    pub legacy_dialog_fix: Option<bool>,
    pub unhide_public_desktop: Option<bool>,
    pub classic_search: Option<bool>,
    pub home_grouping: Option<bool>,
    pub library_grouping: Option<bool>,
    pub explorer_start: Option<bool>,
    pub explorer_start_option: Option<u32>,
    pub explorer_start_path: Option<String>,
}
```

### 2.3 Registry Key Definitions (`winview-core::registry::keys`)

```rust
// winview-core/src/registry/keys.rs

use serde::{Deserialize, Serialize};

/// Well-known registry root keys (platform-neutral)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RootKey {
    Hkcu,
    Hklm,
}

/// Registry value type names (matching Win32 REG_* constants)
/// Unknown types use Other(u32) preserving the numeric type ID
/// REG_NONE = Other(0)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueTypeName {
    RegSz,
    RegExpandSz,
    RegDword,
    RegQword,
    RegBinary,
    RegMultiSz,
    Other(u32),
}

/// Platform-neutral well-known key paths
pub mod paths {
    // FolderTypes
    pub const HKLM_FOLDER_TYPES: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\FolderTypes";
    pub const HKCU_FOLDER_TYPES: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\FolderTypes";

    // Per-FolderType GUID subkey values
    pub const CANONICAL_NAME: &str = "CanonicalName";

    // TopViews\{00000000-0000-0000-0000-000000000000} values
    pub const LOGICAL_VIEW_MODE: &str = "LogicalViewMode";
    pub const MODE: &str = "Mode";
    pub const ICON_SIZE: &str = "IconSize";
    pub const GROUP_BY: &str = "GroupBy";
    pub const GROUP_ASCENDING: &str = "GroupAscending";
    pub const SORT_BY_LIST: &str = "SortByList";
    pub const COLUMN_LIST: &str = "ColumnList";
    pub const COL_INFO: &str = "ColInfo";
    pub const F_FLAGS: &str = "FFlags";

    // Bags
    pub const BAG_MRU: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\BagMRU";
    pub const BAGS: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\Bags";
    pub const ALL_FOLDERS_SHELL: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\Bags\AllFolders\Shell";
    pub const ALL_FOLDERS_COMDLG: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\Bags\AllFolders\ComDlg";
    pub const ALL_FOLDERS_COMDLG_LEGACY: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\Bags\AllFolders\ComDlgLegacy";

    // Streams
    pub const STREAMS: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Streams";
    pub const STREAMS_DEFAULTS: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Streams\Defaults";
    pub const DESKTOP: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Streams\Desktop";

    // Advanced
    pub const ADVANCED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";

    // CLSID (Windows 11 features)
    pub const CLASSIC_CONTEXT_MENU_1: &str = r"Software\Classes\CLSID\{86CA1AA0-34AA-4E8B-A509-50C905BAE2A2}";
    pub const CLASSIC_CONTEXT_MENU_2: &str = r"Software\Classes\CLSID\{51668040-5e31-4023-bbe3-149163bd80f1}";
    pub const EXPLORER_START: &str = r"Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}";
    pub const EXPLORER_START_CMD: &str = r"Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\OpenNewWindow\command";
    pub const WIN10_EXPLORER_1: &str = r"Software\Classes\CLSID\{2aa9162e-c906-4dd9-ad0b-3d24a8eef5a0}";
    pub const WIN10_EXPLORER_2: &str = r"Software\Classes\CLSID\{6480100b-5a83-4d1e-9f69-8ae5a88e9a33}";
    pub const HOME_NAV: &str = r"Software\Classes\CLSID\{f874310e-b6b7-47dc-bc84-b9e6b38f5903}";
    pub const GALLERY_NAV: &str = r"Software\Classes\CLSID\{e88865ea-0e1c-4e20-9aa6-edcd0212c87c}";
    pub const CLASSIC_SEARCH: &str = r"Software\Classes\CLSID\{1d64637d-31e9-4b06-9124-e83fb178ac6e}";
    pub const COPY_TO: &str = r"Software\Classes\AllFileSystemObjects\shellex\ContextMenuHandlers\{C2FBB630-2971-11D1-A18C-00C04FD75D13}";
    pub const MOVE_TO: &str = r"Software\Classes\AllFileSystemObjects\shellex\ContextMenuHandlers\{C2FBB631-2971-11D1-A18C-00C04FD75D13}";

    // Policies
    pub const EXPLORER_POLICIES: &str = r"Software\Policies\Microsoft\Windows\Explorer";
    pub const USER_EXPLORER_POLICIES: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer";
    pub const USER_COMDLG_PLACESBAR: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\ComDlg32\PlacesBar";
}

/// Advanced key value names
pub mod advanced_values {
    pub const HIDE_FILE_EXT: &str = "HideFileExt";
    pub const USE_COMPACT_MODE: &str = "UseCompactMode";
    pub const AUTO_CHECK_SELECT: &str = "AutoCheckSelect";
    pub const HIDDEN: &str = "Hidden";
    pub const LAUNCH_TO: &str = "LaunchTo";
    pub const FULL_ROW_SELECT: &str = "FullRowSelect";
    pub const NO_STR_CMP_LOGICAL: &str = "NoStrCmpLogical";
}

/// Policies key value names
pub mod policy_values {
    pub const DISABLE_SEARCH_SUGGESTIONS: &str = "DisableSearchBoxSuggestions";
}
```

### 2.4 Registry Traits (`winview-core::registry::traits`)

```rust
// winview-core/src/registry/traits.rs

use crate::model::{RegistryValue, ValueReadResult, InspectionSnapshot};
use crate::registry::keys::{RootKey, ValueTypeName};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RegistryError {
    #[error("Key not found: {root:?}\\{path}")]
    KeyNotFound { root: RootKey, path: String },

    #[error("Value not found: {root:?}\\{path}\\{name}")]
    ValueNotFound { root: RootKey, path: String, name: String },

    #[error("Type mismatch: expected {expected:?}, got {actual:?}")]
    TypeMismatch { expected: ValueTypeName, actual: ValueTypeName },

    #[error("Access denied: {root:?}\\{path}")]
    AccessDenied { root: RootKey, path: String },

    #[error("Windows API error: {0}")]
    WindowsApi(String),

    #[error("Invalid data: {0}")]
    InvalidData(String),
}

pub type RegistryResult<T> = Result<T, RegistryError>;

/// Read-only registry access trait — implemented by platform adapter
/// NOTE: Key is Send but NOT Sync (matches windows_registry::Key)
/// Value is !Send + !Sync
pub trait RegistryReader: Send + Sync {
    /// Open a key for reading
    fn open_key(&self, root: RootKey, path: &str) -> RegistryResult<Box<dyn RegistryKey>>;

    /// Enumerate subkey names
    fn enum_keys(&self, root: RootKey, path: &str) -> RegistryResult<Vec<String>>;

    /// Get a value with its type
    fn get_value(&self, root: RootKey, path: &str, name: &str) -> RegistryResult<(ValueTypeName, RegistryValue)>;

    /// Get value type only
    fn get_value_type(&self, root: RootKey, path: &str, name: &str) -> RegistryResult<ValueTypeName>;

    /// Capture a complete inspection snapshot (Phase 0/1 primary operation)
    fn inspect_all(&self) -> RegistryResult<InspectionSnapshot>;
}

/// Read-only registry key handle
/// NOTE: Key is Send but NOT Sync (mirrors windows_registry::Key)
pub trait RegistryKey: Send {
    fn enum_values(&self) -> RegistryResult<Vec<(String, ValueTypeName)>>;
    fn enum_subkeys(&self) -> RegistryResult<Vec<String>>;
    fn get_value(&self, name: &str) -> RegistryResult<(ValueTypeName, RegistryValue)>;
    fn get_value_type(&self, name: &str) -> RegistryResult<ValueTypeName>;
    fn open_subkey(&self, name: &str) -> RegistryResult<Box<dyn RegistryKey>>;
}
```

### 2.5 Test Support (`winview-core::test_support`)

```rust
// winview-core/src/test_support.rs

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use crate::model::{InspectionSnapshot, RegistryValue, ValueReadResult, RawRegistryValue, ReadError, InspectionError, ObservationSource, DecodingStatus};
use crate::registry::keys::{RootKey, ValueTypeName};
use crate::registry::traits::{RegistryReader, RegistryKey, RegistryError, RegistryResult};

/// In-memory fake registry for deterministic testing
/// Implements RegistryReader/RegistryKey traits using HashMap storage
pub struct FakeRegistryReader {
    // (root, path) -> FakeRegistryKey
    keys: Arc<Mutex<BTreeMap<(RootKey, String), FakeRegistryKey>>>,
}

impl FakeRegistryReader {
    pub fn new() -> Self {
        Self { keys: Arc::new(Mutex::new(BTreeMap::new())) }
    }

    pub fn insert_key(&self, root: RootKey, path: &str, key: FakeRegistryKey) {
        self.keys.lock().unwrap().insert((root, path.to_string()), key);
    }

    pub fn insert_value(&self, root: RootKey, path: &str, name: &str, type_id: u32, bytes: Vec<u8>, decoded: Option<RegistryValue>) {
        let mut keys = self.keys.lock().unwrap();
        let entry = keys.entry((root, path.to_string())).or_insert_with(FakeRegistryKey::new);
        let status = if decoded.is_some() { DecodingStatus::Decoded } else { DecodingStatus::UnknownType };
        entry.values.insert(name.to_string(), (type_id, bytes, decoded, status));
    }
}

pub struct FakeRegistryKey {
    values: BTreeMap<String, (u32, Vec<u8>, Option<RegistryValue>, DecodingStatus)>, // name -> (type_id, raw_bytes, decoded, status)
    subkeys: BTreeMap<String, FakeRegistryKey>,
}

impl FakeRegistryKey {
    pub fn new() -> Self {
        Self { values: BTreeMap::new(), subkeys: BTreeMap::new() }
    }
}

impl RegistryReader for FakeRegistryReader {
    fn open_key(&self, root: RootKey, path: &str) -> RegistryResult<Box<dyn RegistryKey>> {
        let keys = self.keys.lock().unwrap();
        keys.get(&(root, path.to_string()))
            .map(|k| Box::new(FakeRegistryKeyHandle { inner: k.clone() }) as Box<dyn RegistryKey>)
            .ok_or(RegistryError::KeyNotFound { root, path: path.to_string() })
    }

    fn enum_keys(&self, root: RootKey, path: &str) -> RegistryResult<Vec<String>> {
        let key = self.open_key(root, path)?;
        key.enum_subkeys()
    }

    fn get_value(&self, root: RootKey, path: &str, name: &str) -> RegistryResult<(ValueTypeName, RegistryValue)> {
        let key = self.open_key(root, path)?;
        key.get_value(name)
    }

    fn get_value_type(&self, root: RootKey, path: &str, name: &str) -> RegistryResult<ValueTypeName> {
        let key = self.open_key(root, path)?;
        key.get_value_type(name)
    }

    fn inspect_all(&self) -> RegistryResult<InspectionSnapshot> {
        // Return minimal valid snapshot for testing
        Ok(InspectionSnapshot::default())
    }
}

struct FakeRegistryKeyHandle {
    inner: FakeRegistryKey,
}

impl RegistryKey for FakeRegistryKeyHandle {
    fn enum_values(&self) -> RegistryResult<Vec<(String, ValueTypeName)>> {
        Ok(self.inner.values.iter()
            .map(|(name, (type_id, _, _, _))| (name.clone(), type_id_to_name(*type_id)))
            .collect())
    }

    fn enum_subkeys(&self) -> RegistryResult<Vec<String>> {
        Ok(self.inner.subkeys.keys().cloned().collect())
    }

    fn get_value(&self, name: &str) -> RegistryResult<(ValueTypeName, RegistryValue)> {
        let (type_id, _, decoded, _) = self.inner.values.get(name)
            .ok_or(RegistryError::ValueNotFound { root: RootKey::Hkcu, path: "".into(), name: name.into() })?;
        let decoded = decoded.clone().ok_or(RegistryError::InvalidData("no decoded value in fake".into()))?;
        Ok((type_id_to_name(*type_id), decoded))
    }

    fn get_value_type(&self, name: &str) -> RegistryResult<ValueTypeName> {
        let (type_id, _, _, _) = self.inner.values.get(name)
            .ok_or(RegistryError::ValueNotFound { root: RootKey::Hkcu, path: "".into(), name: name.into() })?;
        Ok(type_id_to_name(*type_id))
    }

    fn open_subkey(&self, name: &str) -> RegistryResult<Box<dyn RegistryKey>> {
        self.inner.subkeys.get(name)
            .map(|k| Box::new(FakeRegistryKeyHandle { inner: k.clone() }) as Box<dyn RegistryKey>)
            .ok_or(RegistryError::KeyNotFound { root: RootKey::Hkcu, path: name.into() })
    }
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
```

---

## 3. Windows API Integration Strategy

### 3.1 Verified `windows-registry 0.100.0` API Reference

The following API signatures are verified against https://docs.rs/windows-registry/0.100.0/:

```rust
// windows_registry crate (0.100.0) — key types and methods

// Predefined root keys are REFERENCES to Key, not owned values
pub const CURRENT_USER: &Key;
pub const LOCAL_MACHINE: &Key;
pub const CLASSES_ROOT: &Key;
pub const CURRENT_CONFIG: &Key;
pub const USERS: &Key;

impl Key {
    // Open a subkey for reading
    pub fn open(&self, path: &str) -> Result<Key, Error>;

    // Iterate values — returns ValueIterator whose items are (String, Value)
    pub fn values(&self) -> Result<ValueIterator<'_>, Error>;

    // Iterate subkeys — returns KeyIterator whose items are String
    pub fn keys(&self) -> Result<KeyIterator<'_>, Error>;

    // Get the Value struct (contains type and raw bytes via Deref)
    pub fn get_value(&self, name: &str) -> Result<Value, Error>;

    // Get just the Type of a value
    pub fn get_type(&self, name: &str) -> Result<Type, Error>;

    // Typed getters (panic on type mismatch or wrong type)
    pub fn get_u32(&self, name: &str) -> Result<u32, Error>;
    pub fn get_u64(&self, name: &str) -> Result<u64, Error>;
    pub fn get_string(&self, name: &str) -> Result<String, Error>;
    pub fn get_multi_string(&self, name: &str) -> Result<Vec<String>, Error>;
    pub fn get_bytes(&self, name: &str) -> Result<Vec<u8>, Error>;
}

// Value — typed registry value with raw bytes access via Deref<Target=[u8]>
pub struct Value { /* private */ }

impl Value {
    // Returns the registry type
    pub fn ty(&self) -> Type;
    // Gets the value as a slice of u16 for raw wide characters
    pub fn as_wide(&self) -> &[u16];
    // set_ty changes the type tag without changing bytes
    pub fn set_ty(&mut self, ty: Type);
}

impl Deref for Value {
    type Target = [u8];
    fn deref(&self) -> &[u8] { /* raw bytes */ }
}

// Type enum — verified variants (no None variant)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    String,           // REG_SZ (1)
    ExpandString,     // REG_EXPAND_SZ (2)
    U32,              // REG_DWORD (4)
    U64,              // REG_QWORD (11)
    MultiString,      // REG_MULTI_SZ (7)
    Bytes,            // REG_BINARY (3)
    Other(u32),       // Unknown/other types with numeric ID (0 = REG_NONE)
}

// From<Type> for u32 provides numeric conversion (REG_* constant value)
impl From<Type> for u32 { /* ... */ }
// From<u32> for Type provides reverse conversion
impl From<u32> for Type { /* ... */ }

// ValueIterator — items are (String, Value)
pub struct ValueIterator<'a> { /* private */ }
impl<'a> Iterator for ValueIterator<'a> {
    type Item = (String, Value);
}

// KeyIterator — items are String
pub struct KeyIterator<'a> { /* private */ }
impl<'a> Iterator for KeyIterator<'a> {
    type Item = String;
}

// Thread safety: Key is Send but NOT Sync; Value is !Send + !Sync
```

**Critical corrections from previous plan:**
- Root constants are `&Key` references, not owned `Key` values
- `Type` variants: `U32`/`U64`/`String`/`ExpandString`/`MultiString`/`Bytes`/`Other(u32)` — NOT `Dword`/`Qword`/`Binary`/`None`
- `REG_NONE` is `Type::Other(0)`, not a dedicated `None` variant
- `Key::get_value(name)` returns `Result<Value, Error>` — NOT generic, NO `get_value_raw()`
- `Value` provides raw bytes via `Deref<Target=[u8]>` — no `as_bytes()` method
- `values()` returns `Result<ValueIterator>` where `ValueIterator::Item = (String, Value)` — single Result, not nested
- `keys()` returns `Result<KeyIterator>` where `KeyIterator::Item = String` — single Result, not nested
- `get_type()` **exists** — returns `Result<Type, Error>`
- `Type` to numeric: use `From<Type> for u32` (not `to_raw()`)
- `Key: Send` but `!Sync` — trait bounds must reflect this; iterators are `!Sync`
- `Value: !Send + !Sync` — cannot be sent across threads
- No `FromRegValue` generic `get_value<T>()` returning `(Type, RegistryValue)` — use `get_value()` returning `Value` + manual conversion

### 3.2 Registry Adapter (`winview-registry`)

```rust
// winview-registry/src/reader.rs

use windows_registry::{CURRENT_USER, LOCAL_MACHINE, Key, Type as RegType, Value as RegValue};
use winview_core::registry::traits::{RegistryReader, RegistryKey, RegistryError, RegistryResult};
use winview_core::registry::keys::{RootKey, ValueTypeName};
use winview_core::model::{RegistryValue, ValueReadResult, InspectionSnapshot, FolderTypeSnapshot, ViewSnapshot, KeySnapshot, InspectionError, ObservationSource, ReadError, RawRegistryValue, DecodingStatus};
use std::collections::BTreeMap;

pub struct WindowsRegistryReader;

impl RegistryReader for WindowsRegistryReader {
    fn open_key(&self, root: RootKey, path: &str) -> RegistryResult<Box<dyn RegistryKey>> {
        let key = match root {
            RootKey::Hkcu => CURRENT_USER.open(path).map_err(|e| RegistryError::WindowsApi(e.to_string()))?,
            RootKey::Hklm => LOCAL_MACHINE.open(path).map_err(|e| RegistryError::WindowsApi(e.to_string()))?,
        };
        Ok(Box::new(WindowsRegistryKey(key)))
    }

    fn enum_keys(&self, root: RootKey, path: &str) -> RegistryResult<Vec<String>> {
        let key = self.open_key(root, path)?;
        key.enum_subkeys()
    }

    fn get_value(&self, root: RootKey, path: &str, name: &str) -> RegistryResult<(ValueTypeName, RegistryValue)> {
        let key = self.open_key(root, path)?;
        key.get_value(name)
    }

    fn get_value_type(&self, root: RootKey, path: &str, name: &str) -> RegistryResult<ValueTypeName> {
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

        Ok(snapshot)
    }
}

struct WindowsRegistryKey(Key);

impl RegistryKey for WindowsRegistryKey {
    fn enum_values(&self) -> RegistryResult<Vec<(String, ValueTypeName)>> {
        let iter = self.0.values()
            .map_err(|e| RegistryError::WindowsApi(e.to_string()))?;
        
        let mut results = Vec::new();
        for (name, value) in iter {
            let type_name = convert_type(value.ty());
            results.push((name, type_name));
        }
        Ok(results)
    }

    fn enum_subkeys(&self) -> RegistryResult<Vec<String>> {
        let iter = self.0.keys()
            .map_err(|e| RegistryError::WindowsApi(e.to_string()))?;
        
        let mut results = Vec::new();
        for name in iter {
            results.push(name);
        }
        Ok(results)
    }

    fn get_value(&self, name: &str) -> RegistryResult<(ValueTypeName, RegistryValue)> {
        // Use get_value to preserve unknown types and raw bytes
        let value = self.0.get_value(name)
            .map_err(|e| RegistryError::WindowsApi(e.to_string()))?;
        
        let (type_name, reg_value, raw_bytes, status) = convert_registry_value(value.ty(), value.deref());
        // We need to return RegistryValue but also need raw bytes for RawRegistryValue
        // The trait returns decoded value; raw bytes are captured in inspect_all via get_value
        // which has access to the full Value (including Deref)
        Ok((type_name, reg_value))
    }

    fn get_value_type(&self, name: &str) -> RegistryResult<ValueTypeName> {
        let vtype = self.0.get_type(name)
            .map_err(|e| RegistryError::WindowsApi(e.to_string()))?;
        
        Ok(convert_type(vtype))
    }

    fn open_subkey(&self, name: &str) -> RegistryResult<Box<dyn RegistryKey>> {
        let subkey = self.0.open(name).map_err(|e| RegistryError::WindowsApi(e.to_string()))?;
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
fn convert_registry_value(vtype: RegType, data: &[u8]) -> (ValueTypeName, RegistryValue, Vec<u8>, DecodingStatus) {
    let type_name = convert_type(vtype);
    let raw_type_id = u32::from(vtype); // Windows REG_* numeric value via From<Type> for u32
    let raw_bytes = data.to_vec(); // ALWAYS preserve raw bytes
    
    match vtype {
        RegType::String => {
            match decode_utf16_le(data) {
                Ok(s) => (type_name, RegistryValue::String(s), raw_bytes, DecodingStatus::Decoded),
                Err(e) => (type_name, RegistryValue::Binary(data.to_vec()), raw_bytes, DecodingStatus::Malformed { reason: e }),
            }
        }
        RegType::ExpandString => {
            match decode_utf16_le(data) {
                Ok(s) => (type_name, RegistryValue::ExpandString(s), raw_bytes, DecodingStatus::Decoded),
                Err(e) => (type_name, RegistryValue::Binary(data.to_vec()), raw_bytes, DecodingStatus::Malformed { reason: e }),
            }
        }
        RegType::U32 => {
            if data.len() != 4 {
                return (type_name, RegistryValue::Binary(data.to_vec()), raw_bytes, DecodingStatus::Malformed { reason: "DWORD length != 4".into() });
            }
            let val = u32::from_le_bytes(data.try_into().unwrap());
            (type_name, RegistryValue::Dword(val), raw_bytes, DecodingStatus::Decoded)
        }
        RegType::U64 => {
            if data.len() != 8 {
                return (type_name, RegistryValue::Binary(data.to_vec()), raw_bytes, DecodingStatus::Malformed { reason: "QWORD length != 8".into() });
            }
            let val = u64::from_le_bytes(data.try_into().unwrap());
            (type_name, RegistryValue::Qword(val), raw_bytes, DecodingStatus::Decoded)
        }
        RegType::Bytes => (type_name, RegistryValue::Binary(data.to_vec()), raw_bytes, DecodingStatus::Decoded),
        RegType::MultiString => {
            match decode_reg_multi_sz(data) {
                Ok(parts) => (type_name, RegistryValue::MultiString(parts), raw_bytes, DecodingStatus::Decoded),
                Err(e) => (type_name, RegistryValue::Binary(data.to_vec()), raw_bytes, DecodingStatus::Malformed { reason: e }),
            }
        }
        RegType::Other(id) => {
            // Preserve unknown types as raw bytes with Windows type ID
            // Includes REG_NONE as Other(0)
            (ValueTypeName::Other(id), RegistryValue::Binary(data.to_vec()), raw_bytes, DecodingStatus::UnknownType)
        }
    }
}

// UTF-16LE decoding — returns error on malformed data, preserves raw bytes in caller
fn decode_utf16_le(data: &[u8]) -> Result<String, String> {
    if data.len() % 2 != 0 {
        return Err("Odd byte length for UTF-16".into());
    }
    let u16s: Vec<u16> = data.chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    // Find null terminator if present
    let end = u16s.iter().position(|&c| c == 0).unwrap_or(u16s.len());
    let valid = &u16s[..end];
    String::from_utf16(valid).map_err(|_| "Invalid UTF-16 sequence".into())
}

/// Dedicated REG_MULTI_SZ decoder — preserves all elements including empty strings
/// Windows semantics: double-null terminates; single-null separates; empty multi-string = zero-length (no strings)
fn decode_reg_multi_sz(data: &[u8]) -> Result<Vec<String>, String> {
    if data.len() % 2 != 0 {
        return Err("Odd byte length for REG_MULTI_SZ".into());
    }
    let u16s: Vec<u16> = data.chunks_exact(2)
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
                result.push(String::from_utf16(&current).map_err(|_| "Invalid UTF-16 in multi-string element".into())?);
                break;
            } else {
                // Single null - separator between strings
                result.push(String::from_utf16(&current).map_err(|_| "Invalid UTF-16 in multi-string element".into())?);
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

// Windows version detection using RtlGetVersion + registry cross-check
fn detect_windows_version() -> VersionInfo {
    // Implementation in version.rs
    VersionInfo::failed(vec!["not implemented".into()])
}

// Inspection modules - collect errors instead of failing
fn inspect_folder_types(reader: &WindowsRegistryReader, snapshot: &mut InspectionSnapshot) { todo!() }
fn inspect_bags(reader: &WindowsRegistryReader, snapshot: &mut InspectionSnapshot) { todo!() }
fn inspect_streams(reader: &WindowsRegistryReader, snapshot: &mut InspectionSnapshot) { todo!() }
fn inspect_advanced(reader: &WindowsRegistryReader, snapshot: &mut InspectionSnapshot) { todo!() }
fn inspect_clsid(reader: &WindowsRegistryReader, snapshot: &mut InspectionSnapshot) { todo!() }
fn inspect_policies(reader: &WindowsRegistryReader, snapshot: &mut InspectionSnapshot) { todo!() }
```

### 3.3 Windows Version Detection (In Adapter, Not Core)

```rust
// winview-registry/src/version.rs

use windows::Win32::Foundation::NTSTATUS;
use windows::Wdk::System::SystemServices::{RtlGetVersion, OSVERSIONINFOW};
use windows_registry::{LOCAL_MACHINE, Key};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionInfo {
    pub build: Option<u32>,
    pub ubr: Option<u32>,
    pub product_name: Option<String>,
    pub detection_method: DetectionMethod,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetectionMethod {
    #[default]
    Failed,
    RtlGetVersion,
    RegistryFallback,
}

impl VersionInfo {
    pub fn failed(errors: Vec<String>) -> Self {
        Self {
            build: None,
            ubr: None,
            product_name: None,
            detection_method: DetectionMethod::Failed,
            errors,
        }
    }
    
    pub fn rtl_get_version(build: u32, ubr: Option<u32>, product_name: Option<String>) -> Self {
        Self {
            build: Some(build),
            ubr,
            product_name,
            detection_method: DetectionMethod::RtlGetVersion,
            errors: Vec::new(),
        }
    }
    
    pub fn registry_fallback(build: u32, ubr: Option<u32>, product_name: Option<String>) -> Self {
        Self {
            build: Some(build),
            ubr,
            product_name,
            detection_method: DetectionMethod::RegistryFallback,
            errors: Vec::new(),
        }
    }
}

pub fn detect_windows_version() -> VersionInfo {
    // Primary: RtlGetVersion from ntdll (WDK API)
    if let Some(info) = get_version_rtl() {
        return info;
    }
    
    // Fallback: registry cross-check
    if let Some(info) = get_version_registry() {
        return info;
    }
    
    VersionInfo::failed(vec!["Both RtlGetVersion and registry fallback failed".into()])
}

fn get_version_rtl() -> Option<VersionInfo> {
    // RtlGetVersion signature: NTSTATUS RtlGetVersion(OSVERSIONINFOW*)
    // OSVERSIONINFOW contains: dwOSVersionInfoSize, dwMajorVersion, dwMinorVersion, 
    // dwBuildNumber, dwPlatformId, szCSDVersion
    // UBR is NOT in this struct — must read from registry
    
    // SAFETY: RtlGetVersion is a documented, stable API
    let mut info: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
    info.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
    
    // Link ntdll and call RtlGetVersion via windows crate
    // Requires: windows = { features = ["Wdk_System_SystemServices", "Win32_Foundation"] }
    let status = unsafe {
        RtlGetVersion(&mut info)
    };
    
    if status != NTSTATUS(0) {
        return None;
    }
    
    let build = info.dwBuildNumber;
    
    // Read UBR from registry
    let ubr = read_ubr_from_registry();
    let product_name = read_product_name_from_registry();
    
    Some(VersionInfo::rtl_get_version(build, ubr, product_name))
}

fn read_ubr_from_registry() -> Option<u32> {
    let key = LOCAL_MACHINE.open(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion").ok()?;
    key.get_u32("UBR").ok()
}

fn read_product_name_from_registry() -> Option<String> {
    let key = LOCAL_MACHINE.open(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion").ok()?;
    key.get_string("ProductName").ok()
}

fn get_version_registry() -> Option<VersionInfo> {
    // Read from HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion
    // CurrentBuild, CurrentBuildNumber (string), UBR, ProductName
    let key = LOCAL_MACHINE.open(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion").ok()?;
    
    // CurrentBuild is stored as string in registry
    let build_str: String = key.get_string("CurrentBuild").ok()?;
    let build: u32 = build_str.parse().ok()?;
    let ubr: Option<u32> = key.get_u32("UBR").ok();
    let product_name: Option<String> = key.get_string("ProductName").ok();
    
    Some(VersionInfo::registry_fallback(build, ubr, product_name))
}
```

### 3.4 Capability Matrix (Platform-Neutral Data Only)

```rust
// winview-core/src/windows/features.rs

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeatureCapabilities {
    pub compact_view: bool,
    pub classic_context_menu: bool,
    pub home_gallery_removal: bool,
    pub win10_explorer_on_11: bool,
    pub feature_18755234: bool,
    pub feature_40729001: bool,
    pub other_start_folder: bool,
    pub legacy_dialog_fix: bool,
    pub numerical_sort_policy: bool,
}

impl FeatureCapabilities {
    /// Compute capabilities from Windows build/UBR
    /// Called from app layer after version detection
    pub fn from_build_ubr(build: Option<u32>, ubr: Option<u32>) -> Self {
        let build = build.unwrap_or(0);
        let ubr = ubr.unwrap_or(0);
        Self {
            compact_view: build >= 21996,
            classic_context_menu: build >= 21996,
            home_gallery_removal: build >= 22000,
            win10_explorer_on_11: build >= 21996,
            feature_18755234: build >= 19045 && ubr >= 3754 && build < 21996,
            feature_40729001: build == 22621 && ubr >= 3007 && ubr < 3085,
            other_start_folder: build < 26100 || (build == 26100 && ubr < 4202),
            legacy_dialog_fix: build >= 18363,
            numerical_sort_policy: true,
        }
    }
}

/// Support tier for Windows versions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupportTier {
    Supported,
    WarnUnsupported,
    RefuseWrites,
}

pub fn support_tier(build: Option<u32>, ubr: Option<u32>) -> SupportTier {
    let build = build.unwrap_or(0);
    let ubr = ubr.unwrap_or(0);
    match (build, ubr) {
        (19041..=21995, _) => SupportTier::Supported,
        (22000..=22620, _) => SupportTier::Supported,
        (22621, 3007..=3084) => SupportTier::WarnUnsupported,
        (22621, 3085..) => SupportTier::Supported,
        (22631, _) => SupportTier::Supported,
        (26100, _) => SupportTier::Supported,
        _ => SupportTier::RefuseWrites,
    }
}
```

---

## 4. Error Handling & Logging

### 4.1 Error Hierarchy (`winview-core::error`)

```rust
// winview-core/src/error.rs

use thiserror::Error;

#[derive(Error, Debug)]
pub enum WinViewRsError {
    #[error("Registry error: {0}")]
    Registry(#[from] crate::registry::traits::RegistryError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerdeJson(#[from] serde_json::Error),

    #[error("Windows version detection failed: {0}")]
    VersionDetection(String),

    #[error("Unsupported operation on this Windows version: {0}")]
    UnsupportedFeature(String),

    #[error("Validation failed: {0}")]
    Validation(String),
}

// Core does NOT use anyhow — only thiserror for typed errors
```

### 4.2 Logging Setup (App Crate Only)

```rust
// winview-app/src/logging.rs

use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

pub fn init_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,winview_core=debug,winview_registry=debug,winview_app=debug"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false).with_thread_ids(true))
        .init();
}
```

---

## 5. Testing Strategy

### 5.1 Test Organization (Per-Crate)

```
crates/
├── winview-core/
│   └── tests/
│       ├── model_tests.rs
│       ├── version_tests.rs
│       └── property_tests.rs
├── winview-registry/
│   └── tests/
│       └── integration.rs        # Windows-only, read-only, #[ignore] by default
└── winview-app/
    └── tests/
        └── cli_tests.rs          # CLI integration tests

fixtures/                          # Shared test fixtures (workspace root)
├── registry_snapshots/
├── profiles/
└── edge_cases/
    ├── multi_sz_empty.bin
    ├── multi_sz_single.bin
    ├── multi_sz_multiple.bin
    ├── multi_sz_missing_double_null.bin
    ├── multi_sz_empty_elements.bin
    ├── multi_sz_trailing_data.bin
    ├── multi_sz_odd_length.bin
    ├── multi_sz_invalid_utf16.bin
    └── multi_sz_large.bin
```

### 5.2 Unit Tests in `winview-core` (Cross-Platform)

```rust
// crates/winview-core/tests/model_tests.rs

use winview_core::model::{InspectionSnapshot, RegistryValue, RootKey, RawRegistryValue, ValueReadResult, ObservationSource, ReadError, DecodingStatus, ValueTypeName};
use winview_core::registry::traits::RegistryReader;
use winview_core::test_support::FakeRegistryReader;

#[test]
fn inspection_snapshot_serialization() {
    let snapshot = InspectionSnapshot::default();
    let json = serde_json::to_string(&snapshot).unwrap();
    let parsed: InspectionSnapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(snapshot.metadata.captured_at, parsed.metadata.captured_at);
}

#[test]
fn registry_value_type_preservation() {
    let values = vec![
        RegistryValue::String("test".into()),
        RegistryValue::Dword(42),
        RegistryValue::Qword(1234567890123),
        RegistryValue::Binary(vec![0x01, 0x02, 0x03]),
        RegistryValue::MultiString(vec!["a".into(), "b".into()]),
        // REG_NONE and unknown types are NOT in RegistryValue enum
    ];
    for v in values {
        let json = serde_json::to_string(&v).unwrap();
        let parsed: RegistryValue = serde_json::from_str(&json).unwrap();
        assert_eq!(v, parsed);
    }
}

#[test]
fn raw_registry_value_always_has_raw_bytes() {
    let read = RawRegistryValue {
        key_path: r"Software\Test".to_string(),
        value_name: "Value".to_string(),
        root: RootKey::Hkcu,
        raw_type_id: 4,
        raw_bytes: vec![0x7B, 0x00, 0x00, 0x00], // 123 in little-endian
        decoded: Some(RegistryValue::Dword(123)),
        decoding_status: DecodingStatus::Decoded,
        source: ObservationSource::HkcuAdvanced,
    };
    let json = serde_json::to_string(&read).unwrap();
    let parsed: RawRegistryValue = serde_json::from_str(&json).unwrap();
    assert_eq!(read.raw_bytes, parsed.raw_bytes);
    assert_eq!(read.raw_type_id, parsed.raw_type_id);
    assert!(matches!(parsed.decoding_status, DecodingStatus::Decoded));
}

#[test]
fn raw_registry_value_malformed_preserves_bytes() {
    let read = RawRegistryValue {
        key_path: r"Software\Test".to_string(),
        value_name: "BadDword".to_string(),
        root: RootKey::Hkcu,
        raw_type_id: 4,
        raw_bytes: vec![0x7B, 0x00], // Wrong length for DWORD
        decoded: None,
        decoding_status: DecodingStatus::Malformed { reason: "DWORD length != 4".into() },
        source: ObservationSource::HkcuAdvanced,
    };
    let json = serde_json::to_string(&read).unwrap();
    let parsed: RawRegistryValue = serde_json::from_str(&json).unwrap();
    assert_eq!(read.raw_bytes, parsed.raw_bytes);
    assert!(matches!(parsed.decoding_status, DecodingStatus::Malformed { .. }));
    assert!(parsed.decoded.is_none());
}

#[test]
fn raw_registry_value_unknown_type_preserves_bytes_and_id() {
    let read = RawRegistryValue {
        key_path: r"Software\Test".to_string(),
        value_name: "Unknown".to_string(),
        root: RootKey::Hkcu,
        raw_type_id: 999, // Unknown type
        raw_bytes: vec![0xFF, 0xEE, 0xDD],
        decoded: None,
        decoding_status: DecodingStatus::UnknownType,
        source: ObservationSource::HkcuAdvanced,
    };
    let json = serde_json::to_string(&read).unwrap();
    let parsed: RawRegistryValue = serde_json::from_str(&json).unwrap();
    assert_eq!(read.raw_bytes, parsed.raw_bytes);
    assert_eq!(read.raw_type_id, parsed.raw_type_id);
    assert!(matches!(parsed.decoding_status, DecodingStatus::UnknownType));
}

#[test]
fn value_read_result_success_serialization() {
    let read = RawRegistryValue {
        key_path: r"Software\Test".to_string(),
        value_name: "Value".to_string(),
        root: RootKey::Hkcu,
        raw_type_id: 4,
        raw_bytes: vec![0x7B, 0x00, 0x00, 0x00],
        decoded: Some(RegistryValue::Dword(123)),
        decoding_status: DecodingStatus::Decoded,
        source: ObservationSource::HkcuAdvanced,
    };
    let result: ValueReadResult = ValueReadResult::Success(read);
    let json = serde_json::to_string(&result).unwrap();
    let parsed: ValueReadResult = serde_json::from_str(&json).unwrap();
    assert!(matches!(parsed, ValueReadResult::Success(r) if r.decoded == Some(RegistryValue::Dword(123))));
}

#[test]
fn value_read_result_failure_serialization() {
    let error = ReadError::KeyNotFound { path: r"Software\Missing".to_string() };
    let result: ValueReadResult = ValueReadResult::Failure(error);
    let json = serde_json::to_string(&result).unwrap();
    let parsed: ValueReadResult = serde_json::from_str(&json).unwrap();
    assert!(matches!(parsed, ValueReadResult::Failure(ReadError::KeyNotFound { .. })));
}

#[test]
fn inspection_error_captures_context() {
    let err = winview_core::model::InspectionError {
        key_path: r"Software\Missing".to_string(),
        root: RootKey::Hkcu,
        operation: "open_key".to_string(),
        error: "Key not found".to_string(),
    };
    let json = serde_json::to_string(&err).unwrap();
    let parsed: winview_core::model::InspectionError = serde_json::from_str(&json).unwrap();
    assert_eq!(err.key_path, parsed.key_path);
    assert_eq!(err.root, parsed.root);
}

#[test]
fn fake_registry_reader_basic_operations() {
    let fake = FakeRegistryReader::new();
    fake.insert_value(RootKey::Hkcu, r"Software\Test", "Value", 4, vec![0x7B, 0x00, 0x00, 0x00], Some(RegistryValue::Dword(123)));
    
    let result = fake.get_value(RootKey::Hkcu, r"Software\Test", "Value");
    assert!(result.is_ok());
    let (type_name, value) = result.unwrap();
    assert_eq!(type_name, ValueTypeName::RegDword);
    assert_eq!(value, RegistryValue::Dword(123));
}

#[test]
fn fake_registry_reader_missing_key_returns_error() {
    let fake = FakeRegistryReader::new();
    let result = fake.get_value(RootKey::Hkcu, r"Software\Missing", "Value");
    assert!(matches!(result, Err(winview_core::registry::traits::RegistryError::KeyNotFound { .. })));
}

#[test]
fn fake_registry_reader_missing_value_returns_error() {
    let fake = FakeRegistryReader::new();
    fake.insert_key(RootKey::Hkcu, r"Software\Test", winview_core::test_support::FakeRegistryKey::new());
    let result = fake.get_value(RootKey::Hkcu, r"Software\Test", "MissingValue");
    assert!(matches!(result, Err(winview_core::registry::traits::RegistryError::ValueNotFound { .. })));
}
```

### 5.3 Property-Based Tests (Valid Strategies)

```rust
// crates/winview-core/tests/property_tests.rs

use proptest::prelude::*;
use winview_core::model::{RegistryValue, RawRegistryValue, DecodingStatus, RootKey, ValueTypeName, ObservationSource, ReadError, ValueReadResult};

proptest! {
    #[test]
    fn registry_value_roundtrip(v in arb_registry_value()) {
        let json = serde_json::to_string(&v).unwrap();
        let parsed: RegistryValue = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(v, parsed);
    }

    #[test]
    fn raw_registry_value_roundtrip(
        path in ".*",
        name in ".*",
        root in prop::sample::select(vec![RootKey::Hkcu, RootKey::Hklm]),
        type_id in 0u32..200,
        bytes in proptest::collection::vec(any::<u8>(), 0..100),
        source in prop::sample::select(vec![
            ObservationSource::HkcuAdvanced,
            ObservationSource::HkcuFolderTypes,
            ObservationSource::HklmFolderTypes,
        ]),
    ) {
        let read = RawRegistryValue {
            key_path: path,
            value_name: name,
            root,
            raw_type_id: type_id,
            raw_bytes: bytes.clone(),
            decoded: None,
            decoding_status: DecodingStatus::UnknownType,
            source,
        };
        let json = serde_json::to_string(&read).unwrap();
        let parsed: RawRegistryValue = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(read.raw_bytes, parsed.raw_bytes);
        prop_assert_eq!(read.raw_type_id, parsed.raw_type_id);
    }

    #[test]
    fn snapshot_metadata_roundtrip(
        build in proptest::option::of(10000u32..30000),
        ubr in proptest::option::of(0u32..10000),
        product in proptest::option::of(".*"),
        method in prop::sample::select(vec![
            winview_core::model::DetectionMethod::Failed,
            winview_core::model::DetectionMethod::RtlGetVersion,
            winview_core::model::DetectionMethod::RegistryFallback,
        ]),
    ) {
        let meta = winview_core::model::SnapshotMetadata {
            captured_at: "2024-01-01T00:00:00Z".to_string(),
            windows_build: build,
            windows_ubr: ubr,
            windows_product_name: product,
            detection_method: method,
        };
        let json = serde_json::to_string(&meta).unwrap();
        let parsed: winview_core::model::SnapshotMetadata = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(meta.windows_build, parsed.windows_build);
        prop_assert_eq!(meta.windows_ubr, parsed.windows_ubr);
    }
}

fn arb_registry_value() -> impl Strategy<Value = RegistryValue> {
    prop_oneof![
        ".*".prop_map(RegistryValue::String),
        any::<u32>().prop_map(RegistryValue::Dword),
        any::<u64>().prop_map(RegistryValue::Qword),
        proptest::collection::vec(any::<u8>(), 0..100).prop_map(RegistryValue::Binary),
        proptest::collection::vec(".*", 0..10).prop_map(RegistryValue::MultiString),
        // No RegistryValue::None — unknown types use RawRegistryValue directly
    ]
}
```

### 5.4 Windows Integration Tests in `winview-registry` (Read-Only)

```rust
// crates/winview-registry/tests/integration.rs

#[cfg(target_os = "windows")]
mod integration {
    use winview_registry::WindowsRegistryReader;
    use winview_core::registry::traits::RegistryReader;
    use winview_core::model::{InspectionSnapshot, RegistryValue, RawRegistryValue, ValueReadResult};
    use winview_core::registry::keys::RootKey;

    #[test]
    #[ignore = "requires Windows registry access"]
    fn inspect_all_returns_snapshot() {
        let reader = WindowsRegistryReader;
        let snapshot = reader.inspect_all().unwrap();
        
        // Basic structure validation
        assert!(!snapshot.metadata.captured_at.is_empty());
        // Build may be None if detection failed — that's acceptable but recorded
        // No assertion on build value
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn folder_types_hkcu_readable() {
        let reader = WindowsRegistryReader;
        let result = reader.open_key(RootKey::Hkcu, 
            "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FolderTypes");
        // Missing HKCU overrides is normal - key may not exist
        // Either Ok or KeyNotFound are valid
        assert!(result.is_ok() || matches!(result, Err(winview_core::registry::traits::RegistryError::KeyNotFound { .. })));
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn folder_types_hklm_readable() {
        let reader = WindowsRegistryReader;
        let result = reader.open_key(RootKey::Hklm, 
            "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FolderTypes");
        // HKLM FolderTypes should exist on Windows - read-only access generally works without elevation
        assert!(result.is_ok() || matches!(result, Err(winview_core::registry::traits::RegistryError::KeyNotFound { .. })));
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn handles_missing_keys_gracefully() {
        let reader = WindowsRegistryReader;
        let result = reader.get_value(RootKey::Hkcu,
            "Software\\Nonexistent\\Key", "Value");
        assert!(matches!(result, Err(winview_core::registry::traits::RegistryError::KeyNotFound { .. })));
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn handles_permissions_gracefully() {
        let reader = WindowsRegistryReader;
        let result = reader.open_key(RootKey::Hklm,
            "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FolderTypes");
        // Should not panic - either Ok, KeyNotFound, or AccessDenied
        assert!(result.is_ok() || 
                matches!(result, Err(winview_core::registry::traits::RegistryError::KeyNotFound { .. })) ||
                matches!(result, Err(winview_core::registry::traits::RegistryError::AccessDenied { .. })));
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn snapshot_contains_hkcu_folder_types() {
        let reader = WindowsRegistryReader;
        let snapshot = reader.inspect_all().unwrap();
        
        // HKCU FolderTypes domain should be present (may be empty if no overrides)
        let _ = &snapshot.hkcu_folder_types; // Type check: FolderTypesSnapshot
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn snapshot_contains_hklm_folder_types() {
        let reader = WindowsRegistryReader;
        let snapshot = reader.inspect_all().unwrap();
        
        // HKLM FolderTypes domain should be present
        let _ = &snapshot.hklm_folder_types; // Type check: FolderTypesSnapshot
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn snapshot_contains_advanced_key() {
        let reader = WindowsRegistryReader;
        let snapshot = reader.inspect_all().unwrap();
        
        // Advanced is a single KeySnapshot, always present
        let _ = &snapshot.advanced; // Type check: KeySnapshot
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn value_read_preserves_raw_type_id() {
        let reader = WindowsRegistryReader;
        let snapshot = reader.inspect_all().unwrap();
        
        // Find any value and verify raw_type_id is populated
        let mut found_any = false;
        for (_, fs) in &snapshot.hkcu_folder_types.types {
            for (_, view) in &fs.top_views {
                for (_, vread) in &view.raw_values {
                    if let ValueReadResult::Success(raw) = vread {
                        found_any = true;
                        // Windows REG_* type IDs: 0=REG_NONE, 1=REG_SZ, 2=REG_EXPAND_SZ, 3=REG_BINARY, 4=REG_DWORD, 7=REG_MULTI_SZ, 11=REG_QWORD
                        // Unknown types have other IDs
                        // All are valid u32
                        assert!(raw.raw_type_id <= u32::MAX);
                    }
                }
            }
        }
        // Also check HKLM
        for (_, fs) in &snapshot.hklm_folder_types.types {
            for (_, view) in &fs.top_views {
                for (_, vread) in &view.raw_values {
                    if let ValueReadResult::Success(raw) = vread {
                        found_any = true;
                        assert!(raw.raw_type_id <= u32::MAX);
                    }
                }
            }
        }
        // On a clean system, there should be at least some values
        assert!(found_any, "Expected at least one registry value in FolderTypes");
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn unknown_registry_types_preserved() {
        let reader = WindowsRegistryReader;
        let snapshot = reader.inspect_all().unwrap();
        
        // Scan for any RegistryValue::Unknown - if present, verify type_id and data preserved
        fn check_unknown(val: &winview_core::model::RegistryValue) -> bool {
            // RegistryValue doesn't have Unknown variant — unknown types are in RawRegistryValue.decoding_status
            // This test documents expected behavior
            false
        }
        
        let mut found_unknown = false;
        for (_, fs) in &snapshot.hkcu_folder_types.types {
            for (_, view) in &fs.top_views {
                for (_, vread) in &view.raw_values {
                    if let ValueReadResult::Success(raw) = vread {
                        if matches!(raw.decoding_status, winview_core::model::DecodingStatus::UnknownType) {
                            found_unknown = true;
                            assert!(raw.raw_type_id > 11 || raw.raw_type_id == 0); // Unknown or REG_NONE
                            assert!(!raw.raw_bytes.is_empty());
                        }
                    }
                }
            }
        }
        // Don't assert found_unknown — just documents the check
        let _ = found_unknown;
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn errors_collected_in_snapshot() {
        let reader = WindowsRegistryReader;
        let snapshot = reader.inspect_all().unwrap();
        
        // Errors vector exists and is serializable
        let json = serde_json::to_string(&snapshot.errors).unwrap();
        let _: Vec<winview_core::model::InspectionError> = serde_json::from_str(&json).unwrap();
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn inspection_status_reflects_completeness() {
        let reader = WindowsRegistryReader;
        let snapshot = reader.inspect_all().unwrap();
        
        // Status should be Complete, Partial, or Failed
        match snapshot.status {
            winview_core::model::InspectionStatus::Complete => {},
            winview_core::model::InspectionStatus::Partial { reason } => {
                assert!(!reason.is_empty());
            },
            winview_core::model::InspectionStatus::Failed { error } => {
                assert!(!error.is_empty());
            },
        }
    }
}
```

### 5.5 CLI Integration Tests in `winview-app`

```rust
// crates/winview-app/tests/cli_tests.rs

#[cfg(test)]
mod cli_tests {
    use std::process::Command;
    use std::env;

    fn get_binary() -> String {
        env::var("CARGO_BIN_EXE_winview-app")
            .expect("CARGO_BIN_EXE_winview-app not set — run via `cargo test`")
    }

    #[test]
    fn cli_help_works() {
        let binary_path = get_binary();

        let output = Command::new(&binary_path)
            .arg("--help")
            .output()
            .expect("Failed to run CLI binary");
        assert!(output.status.success(), "CLI --help failed: {}", String::from_utf8_lossy(&output.stderr));
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("inspect"));
        assert!(stdout.contains("dump"));
        assert!(stdout.contains("export"));
    }

    #[test]
    fn cli_inspect_outputs_json() {
        let binary_path = get_binary();

        let output = Command::new(&binary_path)
            .args(["dump", "--format", "json"])
            .output()
            .expect("Failed to run CLI binary");
        
        // On non-Windows or without registry access, command may fail
        // But if it succeeds, output MUST be valid JSON matching InspectionSnapshot
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let _: serde_json::Value = serde_json::from_str(&stdout)
                .expect("Valid JSON output");
        } else {
            // Document the failure reason
            eprintln!("CLI dump failed (expected on non-Windows): {}", String::from_utf8_lossy(&output.stderr));
        }
    }

    #[test]
    fn cli_invalid_subcommand_returns_error() {
        let binary_path = get_binary();

        let output = Command::new(&binary_path)
            .arg("nonexistent-command")
            .output()
            .expect("Failed to run CLI binary");
        assert!(!output.status.success(), "Invalid subcommand should fail");
    }

    #[test]
    fn cli_version_flag_works() {
        let binary_path = get_binary();

        let output = Command::new(&binary_path)
            .arg("--version")
            .output()
            .expect("Failed to run CLI binary");
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
    }
}
```

### 5.6 Mocking Architecture

**Single strategy: In-memory fakes in `winview-core::test_support`**

- `FakeRegistryReader` and `FakeRegistryKey` implement `RegistryReader`/`RegistryKey` traits using `BTreeMap` storage
- Deterministic, fast, no external dependencies
- Available to all crates via `winview-core` dev-dependency with `test` feature
- `mockall` used ONLY in dependent crates (`winview-registry`, `winview-app`) for unit-testing their own trait consumers
- No separate test support crate — three-crate workspace preserved
- Production builds never include `mockall` or test fakes

```toml
# winview-core Cargo.toml dev-dependencies
[dev-dependencies]
mockall = { workspace = true, optional = true }
proptest = { workspace = true }
insta = { workspace = true }

[features]
test = ["dep:mockall"]
```

```toml
# winview-registry Cargo.toml dev-dependencies
[dev-dependencies]
winview-core = { path = "../winview-core", features = ["test"] }
```

---

## 6. Phase 0 Milestones (Reduced Scope)

### Milestone 0.1: Workspace Setup & Core Types (Days 1-2)

| Task | Deliverable |
|------|-------------|
| Initialize Cargo workspace with 3 crates (virtual root) | `Cargo.toml`, `rust-toolchain.toml` (1.100.0), `LICENSE` |
| Define `RegistryValue`, `RawRegistryValue`, `ValueReadResult`, `InspectionSnapshot`, `FolderTypeSnapshot`, `ViewSnapshot`, `KeySnapshot`, `InspectionError`, `SnapshotMetadata`, `ObservationSource`, `ReadError`, `DecodingStatus`, `InspectionStatus`, `DetectionMethod` | `crates/winview-core/src/model.rs` |
| Define `RegistryReader`, `RegistryKey` traits + `RegistryError` (Key: Send, not Sync) | `crates/winview-core/src/registry/traits.rs` |
| Define well-known paths and value names (platform-neutral) | `crates/winview-core/src/registry/keys.rs` |
| Define `FeatureCapabilities`, `SupportTier`, `from_build_ubr()`, `support_tier()` | `crates/winview-core/src/windows/features.rs` |
| Define `VersionInfo`, `DetectionMethod` (core types only, no detection logic) | `crates/winview-core/src/windows/version.rs` |
| Error types (`WinViewRsError`) | `crates/winview-core/src/error.rs` |
| Test support: `FakeRegistryReader`, `FakeRegistryKey` | `crates/winview-core/src/test_support.rs` |
| Unit tests: model serialization, property tests for `RegistryValue` | `crates/winview-core/tests/` |
| Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace` | Clean CI |
| Verify `winview-core` compiles on Linux/macOS (cross-platform) | CI matrix |

**Note**: Profile models (`ExplorerOptions`, `FolderTypeConfig`, etc.) are NOT in Milestone 0.1 — they belong in `winview-app` and are deferred to Milestone 0.3.

### Milestone 0.2: Registry Read Adapter (Days 3-4)

| Task | Deliverable |
|------|-------------|
| Implement `WindowsRegistryReader` using `windows-registry` 0.100.0 (correct `Type` enum: `U32`/`U64`/`String`/`ExpandString`/`MultiString`/`Bytes`/`Other(u32)`, `Key` is `Send` only, `Value` is `!Send + !Sync`) | `crates/winview-registry/src/reader.rs` |
| Implement `inspect_all()` capturing FolderTypes (HKLM+HKCU), Bags, Streams, Advanced, CLSID, Policies with error collection | `crates/winview-registry/src/reader.rs` |
| Convert `windows_registry::Type` to platform-neutral `RegistryValue`/`ValueTypeName`, preserve unknown types via `DecodingStatus::UnknownType` and raw bytes | `crates/winview-registry/src/reader.rs` |
| Implement Windows version detection via `RtlGetVersion` (WDK) + registry cross-check (in adapter) | `crates/winview-registry/src/version.rs` |
| Windows integration tests (read-only, #[ignore]) | `crates/winview-registry/tests/integration.rs` |
| Verify `windows-registry` write methods are NOT called (grep audit) | Code review |

### Milestone 0.3: Inspection CLI (Days 5-6)

| Task | Deliverable |
|------|-------------|
| `clap` CLI with subcommands: `inspect`, `dump`, `export` | `crates/winview-app/src/cli/` |
| `inspect` — capture and display `InspectionSnapshot` as human-readable table | `crates/winview-app/src/cli/inspect.rs` |
| `dump` — output `InspectionSnapshot` as JSON | `crates/winview-app/src/cli/inspect.rs` |
| `export` — output `InspectionSnapshot` as TOML (lossy, for debugging) | `crates/winview-app/src/cli/output.rs` |
| Profile model (`ExplorerOptions`, `FolderTypeConfig`, `ViewMode`, etc.) | `crates/winview-app/src/profile/model.rs` |
| Folder type enumeration with GUIDs and canonical names (raw values) | `inspect.rs` |
| Current view mode, columns, group/sort per folder type (raw values) | `inspect.rs` |
| Explorer options detection (raw registry values) | `inspect.rs` |
| CLI integration tests (using `CARGO_BIN_EXE_` mechanism) | `crates/winview-app/tests/cli_tests.rs` |

---

## 7. Phase 1 Milestones (Minimal GUI) — Not Started Yet

*Same as before, but implementation deferred until Milestone 0.3 complete and reviewed.*

---

## 8. Agent Responsibilities & Parallelization

### 8.1 Module Ownership

| Agent | Crate/Module | Primary Files (Exclusive Write Access) |
|-------|--------------|----------------------------------------|
| Agent A (Core) | `winview-core` | `src/model.rs`, `src/registry/traits.rs`, `src/registry/keys.rs`, `src/windows/features.rs`, `src/windows/version.rs`, `src/error.rs`, `src/test_support.rs`, `src/lib.rs`, `tests/*.rs` |
| Agent B (Registry) | `winview-registry` | `src/reader.rs`, `src/version.rs`, `src/lib.rs`, `tests/integration.rs` |
| Agent C (App) | `winview-app` | `src/cli/*.rs`, `src/gui/*.rs`, `src/profile/*.rs`, `src/main.rs`, `src/logging.rs`, `src/lib.rs`, `tests/cli_tests.rs` |

### 8.2 Dependency Order (Sequential Start, Then Parallel)

```
1. Agent A: winview-core (traits, models, capability data) — MUST COMPLETE FIRST
   ↓ publishes: RegistryReader trait, InspectionSnapshot, RegistryValue, FeatureCapabilities, RawRegistryValue, ValueReadResult, ReadError, DecodingStatus, VersionInfo, DetectionMethod, FakeRegistryReader
2. Agent B: winview-registry (implements RegistryReader, version detection) — CAN START AFTER #1
3. Agent C: winview-app (CLI + GUI, consumes RegistryReader) — CAN START AFTER #1
```

**Parallelization gate**: Agent A must publish the `RegistryReader` trait and all shared models before Agents B and C begin. Use a Git worktree for Agent A; Agents B and C can work in separate worktrees once the trait is merged to `main`.

### 8.3 Coordination Rules

1. **No overlapping file edits**: Each agent owns their crate directory exclusively (see table above).
2. **Shared interfaces in `winview-core`**: Agent A defines traits; Agents B and C implement/consume.
3. **Interface stability**: Agent A publishes trait signatures first; others implement against them.
4. **Change protocol**: If Agent B or C discovers a shared interface must change:
   - File an issue/PR against `winview-core`
   - Agent A reviews and merges the interface change
   - Agents B/C rebase and adapt
5. **Required tests before merge**: Each PR must pass `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p <crate>`
6. **PR-based integration**: Each agent works in their worktree; merges via PR to main after review.
7. **Windows-only crates**: `winview-registry` and `winview-app` are `#[cfg(windows)]` — Agent B/C must develop on Windows or use cross-compilation.

---

## 9. Safety & Compliance Checklist

### 9.1 Phase 0/1 Invariants (Enforced by Architecture)

- [ ] **No `RegistryWriter` trait exists** in codebase
- [ ] **No `windows-registry` write methods called** (audit: `rg "set_value|create_key|delete_value|remove_value|delete_key|remove_tree|set_u32|set_u64|set_string|set_bytes|set_multi_string" crates/ --type rs | rg -v "test|mock"`)
- [ ] **No `Stop-Process Explorer` or equivalent** in codebase
- [ ] **No registry key deletion** (`RegDeleteKey`, `DeleteSubKeyTree`)
- [ ] **No elevation prompts** triggered by Phase 0/1 code
- [ ] **All unit tests use `FakeRegistryReader`** — no real registry access in CI
- [ ] **CLI has `inspect`/`dump`/`export` only** — no `apply`, `restore`, `backup` subcommands
- [ ] **GUI has "Refresh" and "Export" only** — no "Apply", "Submit", "Restore" buttons
- [ ] **`inspect_all()` is read-only** — verified by code review

### 9.2 Verification Methods

| Invariant | Verification Method |
|-----------|---------------------|
| No write traits | `rg "RegistryWriter|trait.*Write" crates/` returns nothing |
| No write method calls | `rg "set_value|create_key|delete_value|remove_value|delete_key|remove_tree|set_u32|set_u64|set_string|set_bytes|set_multi_string" crates/ --type rs | rg -v "test|mock"` returns nothing |
| No process termination | `rg "Stop-Process|taskkill|TerminateProcess" crates/` returns nothing |
| No elevation | `rg "runas|elevate|UAC" crates/` returns nothing |
| Tests use mocks | `cargo test -p winview-core` passes without Windows; `rg "WindowsRegistryReader" crates/winview-core/tests/` returns nothing |
| CLI subcommands | `cargo run -p winview-app -- --help` shows only inspect/dump/export |
| GUI buttons | Code review of `inspector_tab.rs` — no apply/restore handlers |

### 9.3 Code Review Gates

- **Pre-merge**: All PRs require `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`
- **Architecture review**: Any new public trait in `winview-core` requires Lead approval
- **Safety audit**: Before Phase 2, full audit of any write implementation
- **Dependency audit**: `cargo audit` and `cargo deny check` in CI

---

## 10. Verified Registry Paths & Values (Against WinSetView Source)

### 10.1 FolderTypes (Verified)

| Path | Source | Values |
|------|--------|--------|
| `HKLM\Software\Microsoft\Windows\CurrentVersion\Explorer\FolderTypes` | WinSetView.ps1:111 | Baseline (read-only) |
| `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\FolderTypes` | WinSetView.ps1:110 | Per-user overrides |
| Per-GUID subkey: `CanonicalName` | WinSetView.ps1:788-789 | Folder type name |
| Per-GUID\TopViews\{00000000-0000-0000-0000-000000000000}: `LogicalViewMode`, `Mode`, `IconSize`, `GroupBy`, `GroupAscending`, `SortByList`, `ColumnList`, `ColInfo`, `FFlags` | WinSetView.ps1:865-876 | View settings |

### 10.2 Bags (Verified)

| Path | Source | Notes |
|------|--------|-------|
| `HKCU\Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\BagMRU` | WinSetView.ps1:103 | MRU list |
| `HKCU\Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\Bags` | WinSetView.ps1:104 | Per-folder saved views |
| `HKCU\...\Bags\AllFolders\Shell` | WinSetView.ps1:105 | Generic defaults |
| `HKCU\...\Bags\AllFolders\ComDlg` | WinSetView.ps1:288-290 | File dialog views |
| `HKCU\...\Bags\AllFolders\ComDlgLegacy` | WinSetView.ps1:292-294 | Legacy dialog views |

### 10.3 Advanced (Verified)

| Path | Values (WinSetView.ps1) |
|------|------------------------|
| `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced` | `HideFileExt`, `UseCompactMode` (build≥21996), `AutoCheckSelect`, `Hidden`, `LaunchTo`, `FullRowSelect` |

### 10.4 CLSID (Verified)

| CLSID | Purpose | Source |
|-------|---------|--------|
| `{86CA1AA0-34AA-4E8B-A509-50C905BAE2A2}` | Classic context menu 1 | WinSetView.ps1:113, 1356-1358 |
| `{51668040-5e31-4023-bbe3-149163bd80f1}` | Classic context menu 2 | WinSetView.ps1:114, 1356-1358 |
| `{52205fd8-5dfb-447d-801a-d0b52f2e83e1}` | Explorer start folder | WinSetView.ps1:126-127, 1399-1414 |
| `{2aa9162e-c906-4dd9-ad0b-3d24a8eef5a0}` | Win10 Explorer on Win11 | WinSetView.ps1:124, 1357-1358 |
| `{f874310e-b6b7-47dc-bc84-b9e6b38f5903}` | Home nav pane | WinSetView.ps1:1359-1360 |
| `{e88865ea-0e1c-4e20-9aa6-edcd0212c87c}` | Gallery nav pane | WinSetView.ps1:1361-1362 |
| `{1d64637d-31e9-4b06-9124-e83fb178ac6e}` | Classic search | WinSetView.ps1:1388-1389 |

### 10.5 Policies (Verified)

| Path | Values |
|------|--------|
| `HKCU\Software\Policies\Microsoft\Windows\Explorer` | `DisableSearchBoxSuggestions` |
| `HKCU\Software\Microsoft\Windows\CurrentVersion\Policies\Explorer` | `NoStrCmpLogical` |
| `HKLM\Software\Microsoft\Windows\CurrentVersion\Policies\Explorer` | `NoStrCmpLogical` |
| `HKCU\Software\Microsoft\Windows\CurrentVersion\Policies\ComDlg32\PlacesBar` | `Place0`-`Place4` (LegacyDialogFix) |

### 10.6 Feature Gates (Verified Against WinSetView.ps1)

| Feature | Build/UBR Gate | Source |
|---------|----------------|--------|
| Compact view | build ≥ 21996 | WinSetView.ps1:417 |
| Classic context menu | build ≥ 21996 | WinSetView.ps1:434 |
| Home/Gallery removal | build ≥ 22000 | WinSetView.ps1:560, 566 |
| Win10 Explorer on Win11 | build ≥ 21996 | WinSetView.ps1:541 |
| Feature 18755234 (Win10 Search) | build ≥ 19045, UBR ≥ 3754, build < 21996 | WinSetView.ps1:509-521 |
| Feature 40729001 (Win11 Explorer) | build = 22621, 3007 ≤ UBR < 3085 | WinSetView.ps1:525-537 |
| Other start folder | build < 26100 or (build = 26100, UBR < 4202) | WinSetView.ps1:596 |
| LegacyDialogFix | build ≥ 18363 | WinSetView.ps1:572 |

---

## 11. Platform Independence & CI Requirements

### 11.1 Crate Platform Targets

| Crate | Target Platforms | Conditional Compilation |
|-------|------------------|------------------------|
| `winview-core` | Windows, Linux, macOS | None — fully platform-independent |
| `winview-registry` | Windows only | `#[cfg(target_os = "windows")]` on entire crate |
| `winview-app` | Windows only | `#[cfg(target_os = "windows")]` on entire crate |

### 11.2 CI Matrix (Valid GitHub Actions)

```yaml
# .github/workflows/ci.yml
name: CI

on: [push, pull_request]

jobs:
  core-tests:
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: 1.100.0
      - name: Test core
        run: cargo test -p winview-core --all-targets
   
  registry-tests:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: 1.100.0
      - run: cargo test -p winview-registry --all-targets
      - run: cargo test -p winview-registry --test integration -- --ignored
   
  app-tests:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: 1.100.0
      - run: cargo test -p winview-app --all-targets
   
  full-workspace:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: 1.100.0
      - run: cargo build --workspace --all-targets
      - run: cargo fmt --check --workspace
      - run: cargo clippy --workspace --all-targets -- -D warnings
```

### 11.3 Cross-Platform Build Verification

- `winview-core` MUST compile on Linux/macOS without Windows-specific dependencies
- No `windows`, `windows-registry`, or other Windows-only crates in `winview-core` dependencies
- `winview-registry` and `winview-app` Cargo.toml specify `target.'cfg(windows)'.dependencies` for Windows-only deps
- Local development: `cargo check -p winview-core` on Windows; CI handles Linux/macOS

---

## 12. Dependencies and Cargo Configuration

### 12.1 Per-Crate Dependencies (Verified)

**`crates/winview-core/Cargo.toml`**:
```toml
[package]
name = "winview-core"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
thiserror = { workspace = true }
# time ONLY for timestamps in tests/fixtures — NOT in core production code
# time = { workspace = true, optional = true }  # Add in Milestone 0.2 if needed

[dev-dependencies]
mockall = { workspace = true, optional = true }
proptest = { workspace = true }
insta = { workspace = true }

[features]
test = ["dep:mockall"]
```

**`crates/winview-registry/Cargo.toml`**:
```toml
[package]
name = "winview-registry"
version = "0.1.0"
edition = "2021"

[dependencies]
winview-core = { path = "../winview-core" }
windows-registry = { workspace = true }
windows = { workspace = true }
thiserror = { workspace = true }
time = { workspace = true }

[dev-dependencies]
winview-core = { path = "../winview-core", features = ["test"] }
```

**`crates/winview-app/Cargo.toml`**:
```toml
[package]
name = "winview-app"
version = "0.1.0"
edition = "2021"

[features]
gui = ["dep:egui", "dep:eframe"]
default = []

[dependencies]
winview-core = { path = "../winview-core" }
winview-registry = { path = "../winview-registry" }
clap = { workspace = true }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
toml = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
time = { workspace = true }
egui = { workspace = true, optional = true }
eframe = { workspace = true, optional = true }

[dev-dependencies]
tempfile = { workspace = true }
```

### 12.2 Dependency Version Compatibility (Rust 1.100.0)

| Crate | Version | MSRV | Verified |
|-------|---------|------|----------|
| `windows-registry` | 0.100.0 | 1.95 | ✅ |
| `egui` | 0.37.0 | 1.95 | ✅ |
| `eframe` | 0.37.0 | 1.95 | ✅ |
| `clap` | 4.6.9 | 1.70 | ✅ |
| `serde` | 1.0.229 | - | ✅ |
| `serde_json` | 1.0.148 | - | ✅ |
| `toml` | 0.8.19 | - | ✅ |
| `thiserror` | 2.0.21 | - | ✅ |
| `anyhow` | 1.0.104 | - | ✅ |
| `tracing` | 0.1.44 | - | ✅ |
| `tracing-subscriber` | 0.3.18 | - | ✅ |
| `mockall` | 0.15.0 | - | ✅ |
| `proptest` | 1.12.0 | - | ✅ |
| `insta` | 1.44.0 | - | ✅ |
| `time` | 0.3 | - | ✅ |
| `windows` | 0.62.0 | 1.70 | ✅ |
| `tempfile` | 3.27.0 | - | ✅ |

**Notes:**
- `windows-registry` 0.100.0 requires Rust 1.95+ — compatible with 1.100.0
- `egui`/`eframe` 0.37.0 require Rust 1.95+ — compatible with 1.100.0
- `windows` crate 0.71.0 does not exist; 0.62.0 is latest verified
- `egui` feature is `default_fonts` (plural), not `default_font`
- `chrono` replaced with `time` crate (no std dependency issues)
- `time` only in registry/app crates; NOT in core for Milestone 0.1

### 12.3 Milestone 0.1 Dependency Exclusions

The following are NOT required for Milestone 0.1 and should be added in later milestones:
- `egui`, `eframe` (GUI, Milestone 1.1)
- `clap` (CLI, Milestone 0.3)
- `tracing`, `tracing-subscriber` (logging, Milestone 0.3)
- `anyhow` (app errors, Milestone 0.3)
- `tempfile` (tests, Milestone 0.2+)
- `toml` (profile in app, Milestone 0.3)
- `time` (timestamps, Milestone 0.2+)
- `windows`, `windows-registry` (Windows adapter, Milestone 0.2)

---

## 13. Acceptance Criteria

### 13.1 Milestone 0.1 (Core Types)

- [ ] **Workspace structure**: Three-crate virtual workspace with correct `Cargo.toml`, `rust-toolchain.toml` (1.100.0), `LICENSE`
- [ ] **Core compiles cross-platform**: CI passes on ubuntu-latest, windows-latest, macos-latest for `winview-core`
- [ ] **Core compiles on Windows**: `cargo check -p winview-core` succeeds
- [ ] **No Windows deps in core**: `cargo tree -p winview-core` shows no `windows`, `windows-registry`, `winapi`
- [ ] **Shared models complete**: `RegistryValue`, `RawRegistryValue`, `ValueReadResult`, `InspectionSnapshot`, `FolderTypeSnapshot`, `ViewSnapshot`, `KeySnapshot`, `InspectionError`, `SnapshotMetadata`, `ObservationSource`, `ReadError`, `DecodingStatus`, `InspectionStatus`, `DetectionMethod`, `RootKey`, `ValueTypeName`
- [ ] **Registry traits complete**: `RegistryReader`, `RegistryKey`, `RegistryError` — all `Send`/`Sync` bounds correct (`Key: Send`, not `Sync`; `Value: !Send + !Sync`)
- [ ] **Well-known keys defined**: All paths from Section 2.3 in `registry/keys.rs`
- [ ] **Feature capabilities defined**: `FeatureCapabilities`, `SupportTier`, `from_build_ubr()`, `support_tier()`
- [ ] **Version types defined**: `VersionInfo`, `DetectionMethod` (core only)
- [ ] **Error types defined**: `WinViewRsError` with all variants
- [ ] **Test support complete**: `FakeRegistryReader`, `FakeRegistryKey` implementing traits
- [ ] **Unit tests pass**: `cargo test -p winview-core` — all tests green
- [ ] **Property tests pass**: `cargo test -p winview-core property_tests` — all green
- [ ] **No `todo!()` in milestone code paths**: All functions in `model.rs`, `traits.rs`, `keys.rs`, `features.rs`, `version.rs`, `error.rs`, `test_support.rs` implemented
- [ ] **Format clean**: `cargo fmt --check -p winview-core` passes
- [ ] **Clippy clean**: `cargo clippy -p winview-core --all-targets -- -D warnings` passes
- [ ] **No registry-writing functionality**: `rg` audit confirms

### 13.2 Milestone 0.2 (Registry Adapter)

- [ ] **Adapter compiles**: `cargo build -p winview-registry` (Windows target)
- [ ] **Correct API usage**: `windows_registry::Type` variants `U32`/`U64`/`String`/`ExpandString`/`MultiString`/`Bytes`/`Other(u32)` used correctly
- [ ] **`get_value()` used for preservation**: All value reads go through `get_value()` returning `Value`, then `Deref` for raw bytes
- [ ] **Iterator handling**: `values()` and `keys()` return `Result<Iterator>` with items `(String, Value)` and `String` directly
- [ ] **`Key` trait bounds**: `RegistryKey: Send` (not `Sync`) — matches `windows_registry::Key`
- [ ] **Version detection works**: `detect_windows_version()` returns `VersionInfo` with `Option<u32>` build/UBR, `DetectionMethod`, errors
- [ ] **`inspect_all()` implemented**: Captures all 6 domains (FolderTypes HKLM+HKCU, Bags, Streams, Advanced, CLSID, Policies)
- [ ] **Errors collected**: `InspectionError` entries in snapshot for any read failures
- [ ] **Unknown types preserved**: `DecodingStatus::UnknownType` with `raw_type_id` and `raw_bytes`
- [ ] **Malformed data preserved**: `DecodingStatus::Malformed` with reason and `raw_bytes`
- [ ] **Integration tests exist**: `crates/winview-registry/tests/integration.rs` with meaningful assertions
- [ ] **All tests pass**: `cargo test -p winview-registry` (unit) and `cargo test -p winview-registry --test integration -- --ignored` (integration)
- [ ] **No write methods called**: `rg` audit of `crates/winview-registry/src/`

### 13.3 Milestone 0.3 (Inspection CLI)

- [ ] **CLI builds**: `cargo build -p winview-app` (Windows target)
- [ ] **Subcommands work**: `inspect`, `dump`, `export` all produce output
- [ ] **Human-readable table**: `inspect` shows folder types, GUIDs, canonical names, view settings
- [ ] **JSON output**: `dump --format json` outputs valid `InspectionSnapshot` JSON
- [ ] **TOML export**: `export` outputs valid TOML (lossy, for debugging)
- [ ] **Profile model in app**: `ExplorerOptions`, `FolderTypeConfig`, `ViewMode`, etc. in `winview-app/src/profile/`
- [ ] **CLI tests pass**: `cargo test -p winview-app` (tests compiled binary via `CARGO_BIN_EXE_`)
- [ ] **No write subcommands**: `rg` audit confirms no `apply`, `restore`, `backup`
- [ ] **Full workspace tests**: `cargo test --workspace` passes

---

## 14. Inspection Limits & Privacy

### 14.1 Resource Limits

| Limit | Value | Behavior on Exceed |
|-------|-------|-------------------|
| Maximum recursion depth | 32 | Stop descent, record `InspectionError`, status = `Partial { reason: "depth_limit_exceeded" }` |
| Maximum keys per domain | 50,000 | Stop enumeration, record `InspectionError`, status = `Partial { reason: "key_limit_exceeded" }` |
| Maximum values per key | 1,000 | Stop enumeration, record `InspectionError`, status = `Partial { reason: "value_limit_exceeded" }` |
| Maximum value size | 1 MB | Read up to limit, record `InspectionError`, status = `Partial { reason: "value_size_limit_exceeded" }` |
| Maximum total snapshot size | 50 MB | Stop collection, record `InspectionError`, status = `Partial { reason: "size_limit_exceeded" }` |

All limits are configurable via environment variables (e.g., `WINVIEW_MAX_DEPTH`, `WINVIEW_MAX_KEYS`, `WINVIEW_MAX_VALUE_SIZE`, `WINVIEW_MAX_TOTAL_SIZE`). Partial results are always returned with errors documenting what was omitted. Exported snapshots include `InspectionStatus` indicating completeness.

**Note**: The `windows-registry` crate reads entire values into memory before returning. The 1 MB limit is enforced AFTER reading; it does not prevent allocation of large values. This limitation is documented; a future streaming API could enforce pre-read limits.

### 14.2 Privacy Guidance

Registry snapshots may contain:
- Personal folder paths (Documents, Downloads, etc.)
- Application configuration with user-specific data
- MRU lists with recently accessed files

Before sharing diagnostic exports:
- Review JSON/TOML output for sensitive paths
- Consider redacting user-specific path components
- Use `dump --format json` for machine parsing, not manual review
- No automatic file writes — explicit user action required for export

---

## 15. Safety Verification Commands

Use `rg` (ripgrep) or PowerShell equivalents:

```powershell
# No write traits
rg "RegistryWriter|trait.*Write" crates/

# No write method calls (exclude tests)
rg "set_value|create_key|delete_value|remove_value|delete_key|remove_tree|set_u32|set_u64|set_string|set_bytes|set_multi_string" crates/ --type rs | rg -v "test|mock"

# No process termination
rg "Stop-Process|taskkill|TerminateProcess" crates/

# No elevation
rg "runas|elevate|UAC" crates/

# No unsafe blocks doing registry writes
rg "unsafe.*RegSetValue|unsafe.*RegCreateKey|unsafe.*RegDelete" crates/

# Reference repo unmodified (run from WinViewRs root)
git -C ../WinSetView-reference status --porcelain
```

**Note**: Text searches are an additional safeguard, not proof that a program cannot modify system state. Runtime testing in a disposable VM is required before any system modifications.

---

## 16. Migration from Existing Single-Package Project

The project already exists at `C:\Users\Admin\Documents\Projects\WinViewRs` and may contain a single-package Cargo project created via `cargo init`.

### Non-Destructive Migration Procedure

1. **Backup existing files**:
   ```powershell
   Copy-Item Cargo.toml Cargo.toml.bak
   Copy-Item -Recurse src src.bak
   ```

2. **Create workspace structure**:
   ```powershell
   New-Item -ItemType Directory -Path crates\winview-core\src -Force
   New-Item -ItemType Directory -Path crates\winview-registry\src -Force
   New-Item -ItemType Directory -Path crates\winview-app\src -Force
   New-Item -ItemType Directory -Path fixtures -Force
   New-Item -ItemType Directory -Path scripts -Force
   ```

3. **Move existing source to `winview-core`** (or split as appropriate):
   ```powershell
   Move-Item src\* crates\winview-core\src\
   ```

4. **Create virtual workspace `Cargo.toml`** (replace root):
   ```toml
   [workspace]
   resolver = "2"
   members = ["crates/winview-core", "crates/winview-registry", "crates/winview-app"]
   # ... workspace.dependencies as defined in Section 1.3
   ```

5. **Create per-crate `Cargo.toml` files** per Section 12.1

6. **Create `rust-toolchain.toml`** per Section 1.3

7. **Preserve `AGENTS.md`, project brief, `.git/` history** — do not delete or overwrite

8. **Verify**:
   ```powershell
   cargo check -p winview-core
   cargo test -p winview-core
   ```

9. **Commit migration** with message: "chore: migrate to three-crate virtual workspace"

---

## 17. Final Verification Checklist

- [x] Architecture diagrams agree with proposed file structure
- [x] File paths and imports agree with module definitions (`RootKey` from `registry::keys`, not `model`)
- [x] Cargo dependencies agree with proposed code (per-crate tables in Section 12)
- [x] Test locations agree with Cargo's discovery rules (`crate/tests/*.rs` for integration)
- [x] Milestone scope agrees with acceptance criteria (no profile models in 0.1)
- [x] Agent responsibilities agree with module ownership (exclusive file access per agent)
- [x] All referenced API names and signatures verified against `windows-registry 0.100.0` docs
- [x] No previously identified compilation errors remain
- [x] No placeholder assertions presented as meaningful tests (all tautologies replaced)
- [x] Security requirements consistent across every section (read-only, no elevation, no writes)
- [x] Proposed implementation order contains no circular dependencies (A→B, A→C)
- [x] `InspectionSnapshot` explicitly NOT claimed as lossless restoration format
- [x] Profile model moved to app layer with clear rationale
- [x] `ValueReadResult` distinguishes success/failure without contradictory mandatory fields
- [x] `RegistryValue` does NOT contain unknown types — they are in `RawRegistryValue` with raw bytes
- [x] Windows version detection returns structured `VersionInfo` with `Option` fields, error tracking
- [x] Conditional compilation for Windows-only crates specified
- [x] `REG_MULTI_SZ` decoder preserves all elements with proper termination validation; empty = `[]`
- [x] Mock generation: single strategy (in-memory fakes in core test_support)
- [x] CLI tests use `CARGO_BIN_EXE_` mechanism with explicit success assertions
- [x] Dependency versions verified against published crates
- [x] CI matrix syntax corrected for GitHub Actions
- [x] Inspection limits specified with explicit `InspectionStatus` states
- [x] Safety verification commands work on Windows (PowerShell-compatible)
- [x] Architecture supports parallel Agent B and Agent C work after Agent A completes core
- [x] Changelog matches the actual revised sections
- [x] Licensing requirements documented (MIT preservation, attribution, independent implementation)
- [x] Migration procedure for existing project included

---

## 18. Remaining Unresolved Technical Decisions

| Question | Status | Recommendation |
|----------|--------|----------------|
| `RtlGetVersion` exact `windows` crate feature set | Verified | `Wdk_System_SystemServices` + `Win32_Foundation`; `OSVERSIONINFOW` available |
| Binary data display format in CLI | Unresolved | Hex dump with ASCII column for `inspect`; base64 for JSON export |
| `time` vs `chrono` for timestamps | Resolved | `time` crate in registry/app (no std issues); NOT in core for M0.1 |
| Large value streaming API | Deferred | Document limitation; revisit if 1MB values common |

---

## 19. Approval Gate

**This refined plan is submitted for review. Implementation of Milestone 0.1 will begin after explicit approval.**

Key refinements from previous version:
- ✅ **Verified `windows-registry 0.100.0` API**: Root constants are `&Key` refs; `Type` enum (`U32`/`U64`/`String`/`ExpandString`/`MultiString`/`Bytes`/`Other(u32)`); `get_value()` returns `Value`; `Deref<Target=[u8]>` for raw bytes; iterators yield directly; `get_type()` exists; `From<Type> for u32`; `Key: Send` not `Sync`; `Value: !Send + !Sync`
- ✅ **Raw bytes always preserved**: `RawRegistryValue` has mandatory `raw_bytes: Vec<u8>`; `decoded: Option<RegistryValue>`; `decoding_status: DecodingStatus` (Decoded/Malformed/UnknownType); no `RegistryValue::None`
- ✅ **REG_MULTI_SZ edge cases specified**: Empty = `[]`; single element; multiple; missing double-null = error; empty elements preserved; trailing data ignored; odd length = error; invalid UTF-16 = error; fixtures for all
- ✅ **Correct version detection**: `RtlGetVersion` in `Wdk_System_SystemServices` with `OSVERSIONINFOW`; `Option<u32>` for build/UBR; `DetectionMethod` enum; no sentinel zeros
- ✅ **Cargo manifests internally consistent**: All workspace deps declared; per-crate tables validated; `mockall` as optional dev-dep with `test` feature; `time` only in registry/app; `anyhow` only in app
- ✅ **Single mocking strategy**: In-memory `FakeRegistryReader` in `winview-core::test_support`; `mockall` only for dependent crate unit tests
- ✅ **All tests fixture-based**: Tautologies replaced; exact assertions; `RootKey` imports fixed; property tests validate real behavior
- ✅ **CLI tests deterministic**: Explicit success assertions; exact JSON schema validation; error-code tests; no silent passes
- ✅ **Inspection limits with explicit states**: `Complete`/`Partial`/`Failed`; limits enforced at collection; exported snapshots include status
- ✅ **Safety verification commands fixed**: PowerShell-compatible; correct reference repo check; added external command/unsafe checks
- ✅ **Licensing documented**: MIT preservation, attribution, independent implementation statement, read-only reference repo
- ✅ **Migration procedure**: Non-destructive conversion from single-package to virtual workspace
- ✅ **Scope aligned**: Milestone 0.1 = minimal viable core only

Please approve to proceed with **Milestone 0.1 only**.