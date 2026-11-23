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
    pub created_at: String, // ISO 8601
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
