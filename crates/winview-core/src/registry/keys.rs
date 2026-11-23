// winview-core/src/registry/keys.rs

use serde::{Deserialize, Serialize};

/// Well-known registry root keys (platform-neutral)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
    pub const HKLM_FOLDER_TYPES: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\FolderTypes";
    pub const HKCU_FOLDER_TYPES: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\FolderTypes";

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
    pub const BAG_MRU: &str =
        r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\BagMRU";
    pub const BAGS: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\Bags";
    pub const ALL_FOLDERS_SHELL: &str =
        r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\Bags\AllFolders\Shell";
    pub const ALL_FOLDERS_COMDLG: &str =
        r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\Bags\AllFolders\ComDlg";
    pub const ALL_FOLDERS_COMDLG_LEGACY: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\Bags\AllFolders\ComDlgLegacy";

    // Streams
    pub const STREAMS: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Streams";
    pub const STREAMS_DEFAULTS: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Streams\Defaults";
    pub const DESKTOP: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Streams\Desktop";

    // Advanced
    pub const ADVANCED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";

    // CLSID (Windows 11 features)
    pub const CLASSIC_CONTEXT_MENU_1: &str =
        r"Software\Classes\CLSID\{86CA1AA0-34AA-4E8B-A509-50C905BAE2A2}";
    pub const CLASSIC_CONTEXT_MENU_2: &str =
        r"Software\Classes\CLSID\{51668040-5e31-4023-bbe3-149163bd80f1}";
    pub const EXPLORER_START: &str =
        r"Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}";
    pub const EXPLORER_START_CMD: &str = r"Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\OpenNewWindow\command";
    pub const WIN10_EXPLORER_1: &str =
        r"Software\Classes\CLSID\{2aa9162e-c906-4dd9-ad0b-3d24a8eef5a0}";
    pub const WIN10_EXPLORER_2: &str =
        r"Software\Classes\CLSID\{6480100b-5a83-4d1e-9f69-8ae5a88e9a33}";
    pub const HOME_NAV: &str = r"Software\Classes\CLSID\{f874310e-b6b7-47dc-bc84-b9e6b38f5903}";
    pub const GALLERY_NAV: &str = r"Software\Classes\CLSID\{e88865ea-0e1c-4e20-9aa6-edcd0212c87c}";
    pub const CLASSIC_SEARCH: &str =
        r"Software\Classes\CLSID\{1d64637d-31e9-4b06-9124-e83fb178ac6e}";
    pub const COPY_TO: &str = r"Software\Classes\AllFileSystemObjects\shellex\ContextMenuHandlers\{C2FBB630-2971-11D1-A18C-00C04FD75D13}";
    pub const MOVE_TO: &str = r"Software\Classes\AllFileSystemObjects\shellex\ContextMenuHandlers\{C2FBB631-2971-11D1-A18C-00C04FD75D13}";

    // Policies
    pub const EXPLORER_POLICIES: &str = r"Software\Policies\Microsoft\Windows\Explorer";
    pub const USER_EXPLORER_POLICIES: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer";
    pub const USER_COMDLG_PLACESBAR: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Policies\ComDlg32\PlacesBar";
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
impl ValueTypeName {
    pub fn to_raw_id(&self) -> u32 {
        match self {
            ValueTypeName::RegSz => 1,
            ValueTypeName::RegExpandSz => 2,
            ValueTypeName::RegBinary => 3,
            ValueTypeName::RegDword => 4,
            ValueTypeName::RegMultiSz => 7,
            ValueTypeName::RegQword => 11,
            ValueTypeName::Other(id) => *id,
        }
    }
}

/// Policies key value names
pub mod policy_values {
    pub const DISABLE_SEARCH_SUGGESTIONS: &str = "DisableSearchBoxSuggestions";
}
