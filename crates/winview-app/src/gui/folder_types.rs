// winview-app/src/gui/folder_types.rs

use eframe::egui;

use winview_core::model::{
    FolderTypeSnapshot, FolderTypesSnapshot, InspectionSnapshot, ViewSnapshot,
};

use winview_core::RootKey;

const FOLDERS_PER_PAGE: usize = 25;
const TEXT_PREVIEW_CHARS: usize = 160;

#[derive(Default)]
pub(super) struct FolderTypesIndex {
    hkcu: Vec<String>,
    hklm: Vec<String>,
}

impl FolderTypesIndex {
    pub(super) fn from_snapshot(snapshot: &InspectionSnapshot) -> Self {
        Self {
            hkcu: snapshot.hkcu_folder_types.types.keys().cloned().collect(),

            hklm: snapshot.hklm_folder_types.types.keys().cloned().collect(),
        }
    }

    fn keys(&self, root: RootKey) -> &[String] {
        match root {
            RootKey::Hkcu => &self.hkcu,
            RootKey::Hklm => &self.hklm,
        }
    }
}

fn folder_types_for_root(snapshot: &InspectionSnapshot, root: RootKey) -> &FolderTypesSnapshot {
    match root {
        RootKey::Hkcu => &snapshot.hkcu_folder_types,

        RootKey::Hklm => &snapshot.hklm_folder_types,
    }
}

pub(super) struct FolderBrowser {
    root: RootKey,
    page: usize,
    selected_guid: Option<String>,
    view_index: usize,
    selected_value: Option<String>,
}

impl Default for FolderBrowser {
    fn default() -> Self {
        Self {
            root: RootKey::Hklm,
            page: 0,
            selected_guid: None,
            view_index: 0,
            selected_value: None,
        }
    }
}

impl FolderBrowser {
    fn select_root(&mut self, root: RootKey) {
        if self.root != root {
            self.root = root;
            self.page = 0;
            self.selected_guid = None;
            self.view_index = 0;
            self.selected_value = None;
        }
    }

    fn page_count(entry_count: usize) -> usize {
        entry_count.div_ceil(FOLDERS_PER_PAGE).max(1)
    }

    fn page_range(&mut self, entry_count: usize) -> std::ops::Range<usize> {
        let pages = Self::page_count(entry_count);

        self.page = self.page.min(pages - 1);

        let start = self.page * FOLDERS_PER_PAGE;

        let end = start.saturating_add(FOLDERS_PER_PAGE).min(entry_count);

        start..end
    }

    fn select_page(&mut self, page: usize, entry_count: usize) {
        let next = page.min(Self::page_count(entry_count) - 1);

        if next != self.page {
            self.page = next;
            self.selected_guid = None;
            self.view_index = 0;
            self.selected_value = None;
        }
    }

    fn select_folder(&mut self, guid: &str) {
        if self.selected_guid.as_deref() != Some(guid) {
            self.selected_guid = Some(guid.to_string());

            self.view_index = 0;
            self.selected_value = None;
        }
    }

    fn select_view(&mut self, index: usize) {
        if self.view_index != index {
            self.view_index = index;
            self.selected_value = None;
        }
    }

    fn selected_folder<'a>(
        &self,
        snapshot: &'a InspectionSnapshot,
    ) -> Option<&'a FolderTypeSnapshot> {
        let guid = self.selected_guid.as_deref()?;

        folder_types_for_root(snapshot, self.root).types.get(guid)
    }

    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &InspectionSnapshot,
        index: &FolderTypesIndex,
    ) {
        ui.heading("FolderTypes");

        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.root == RootKey::Hklm, "HKLM - System")
                .clicked()
            {
                self.select_root(RootKey::Hklm);
            }

            if ui
                .selectable_label(self.root == RootKey::Hkcu, "HKCU - User")
                .clicked()
            {
                self.select_root(RootKey::Hkcu);
            }
        });

        let keys = index.keys(self.root);

        let folder_types = folder_types_for_root(snapshot, self.root);

        ui.label(format!("{} captured folder-type records", keys.len(),));

        ui.label(format!(
            "Hive-level inspection errors: {}",
            folder_types.read_errors.len(),
        ));

        ui.small(
            "These are captured registry records, \
             not necessarily effective Explorer settings.",
        );

        if keys.is_empty() {
            ui.label("No folder-type records were captured in this hive.");

            return;
        }

        self.page_range(keys.len());

        let pages = Self::page_count(keys.len());

        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.page > 0, egui::Button::new("Previous"))
                .clicked()
            {
                self.select_page(self.page.saturating_sub(1), keys.len());
            }

            ui.label(format!("Page {} of {}", self.page + 1, pages,));

            if ui
                .add_enabled(self.page + 1 < pages, egui::Button::new("Next"))
                .clicked()
            {
                self.select_page(self.page + 1, keys.len());
            }
        });

        let range = self.page_range(keys.len());

        ui.columns(2, |columns| {
            columns[0].heading("Folder types");

            for guid in &keys[range] {
                let Some(folder) = folder_types.types.get(guid) else {
                    continue;
                };

                let name = text_or_unavailable(folder.canonical_name.as_deref());

                columns[0].push_id(guid.as_str(), |ui| {
                    if ui
                        .selectable_label(
                            self.selected_guid.as_deref() == Some(guid.as_str()),
                            name,
                        )
                        .clicked()
                    {
                        self.select_folder(guid);
                    }

                    ui.small(guid.as_str());
                });
            }

            columns[1].heading("Folder details");

            self.show_details(&mut columns[1], snapshot);
        });
    }

    fn show_details(&mut self, ui: &mut egui::Ui, snapshot: &InspectionSnapshot) {
        let Some(folder) = self.selected_folder(snapshot) else {
            self.selected_value = None;

            ui.label("Select a folder type to inspect its captured settings.");

            return;
        };

        ui.strong(text_or_unavailable(folder.canonical_name.as_deref()));

        ui.label(format!("GUID: {}", folder.guid,));

        ui.label(format!(
            "Folder-level inspection errors: {}",
            folder.read_errors.len(),
        ));

        ui.separator();

        if folder.top_views.is_empty() {
            self.selected_value = None;

            ui.label("No TopViews records were captured.");

            return;
        }

        let count = folder.top_views.len();

        self.select_view(self.view_index.min(count - 1));

        if count > 1 {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.view_index > 0, egui::Button::new("Previous view"))
                    .clicked()
                {
                    self.select_view(self.view_index.saturating_sub(1));
                }

                if ui
                    .add_enabled(self.view_index + 1 < count, egui::Button::new("Next view"))
                    .clicked()
                {
                    self.select_view(self.view_index + 1);
                }

                ui.label(format!("View {} of {}", self.view_index + 1, count,));
            });
        }

        let Some((guid, view)) = folder.top_views.iter().nth(self.view_index) else {
            self.selected_value = None;

            return;
        };

        ui.label(format!("View GUID: {guid}"));

        egui::Grid::new("folder_view_details")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                for (label, value) in view_rows(view) {
                    let selected = self.selected_value.as_deref() == Some(label);

                    if ui.selectable_label(selected, label).clicked() {
                        self.selected_value = Some(label.to_string());
                    }

                    ui.label(value);
                    ui.end_row();
                }
            });

        ui.small(
            "Unavailable fields may be missing, \
             unreadable, omitted, or undecodable.",
        );

        if let Some(name) = self.selected_value.as_deref() {
            match view.raw_values.get(name) {
                Some(observation) => {
                    super::value_details::show_value_details(ui, name, observation);
                }
                None => {
                    ui.separator();
                    ui.heading("Value details");
                    ui.label(name);
                    ui.label("No raw observation was captured for this field.");
                }
            }
        }
    }
}

fn preview_text(value: &str) -> String {
    let mut chars = value.chars();

    let mut preview: String = chars.by_ref().take(TEXT_PREVIEW_CHARS).collect();

    if chars.next().is_some() {
        preview.push('…');
    }

    preview
}

fn text_or_unavailable(value: Option<&str>) -> String {
    match value {
        Some("") => "(empty string)".into(),
        Some(text) => preview_text(text),
        None => "Unavailable".into(),
    }
}

fn number_or_unavailable(value: Option<u32>) -> String {
    value
        .map(|number| number.to_string())
        .unwrap_or_else(|| "Unavailable".into())
}

fn view_rows(view: &ViewSnapshot) -> [(&'static str, String); 9] {
    [
        (
            "LogicalViewMode",
            number_or_unavailable(view.logical_view_mode),
        ),
        ("Mode", number_or_unavailable(view.mode)),
        ("IconSize", number_or_unavailable(view.icon_size)),
        ("GroupBy", text_or_unavailable(view.group_by.as_deref())),
        (
            "GroupAscending",
            number_or_unavailable(view.group_ascending),
        ),
        (
            "SortByList",
            text_or_unavailable(view.sort_by_list.as_deref()),
        ),
        (
            "ColumnList",
            text_or_unavailable(view.column_list.as_deref()),
        ),
        (
            "ColInfo",
            view.col_info
                .as_ref()
                .map(|bytes| format!("{} bytes", bytes.len()))
                .unwrap_or_else(|| "Unavailable".into()),
        ),
        ("FFlags", number_or_unavailable(view.f_flags)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use winview_core::model::{FolderTypeSnapshot, InspectionSnapshot, ViewSnapshot};

    use winview_core::RootKey;

    fn snapshot_with_folder_types() -> InspectionSnapshot {
        let mut snapshot = InspectionSnapshot::default();

        let hkcu_folder = FolderTypeSnapshot {
            guid: "GUID-A".into(),
            canonical_name: Some("User Documents".into()),
            top_views: BTreeMap::new(),
            read_errors: vec![],
        };

        let hklm_folder = FolderTypeSnapshot {
            guid: "GUID-A".into(),
            canonical_name: Some("System Documents".into()),
            top_views: BTreeMap::new(),
            read_errors: vec![],
        };

        let hkcu_folder_b = FolderTypeSnapshot {
            guid: "GUID-B".into(),
            canonical_name: Some("User Pictures".into()),
            top_views: BTreeMap::new(),
            read_errors: vec![],
        };

        let hklm_folder_c = FolderTypeSnapshot {
            guid: "GUID-C".into(),
            canonical_name: Some("System Music".into()),
            top_views: BTreeMap::new(),
            read_errors: vec![],
        };

        snapshot
            .hkcu_folder_types
            .types
            .insert("GUID-A".into(), hkcu_folder);
        snapshot
            .hkcu_folder_types
            .types
            .insert("GUID-B".into(), hkcu_folder_b);
        snapshot
            .hklm_folder_types
            .types
            .insert("GUID-A".into(), hklm_folder);
        snapshot
            .hklm_folder_types
            .types
            .insert("GUID-C".into(), hklm_folder_c);

        snapshot
    }

    #[test]
    fn folder_index_preserves_hive_separation_and_order() {
        let snapshot = snapshot_with_folder_types();
        let index = FolderTypesIndex::from_snapshot(&snapshot);

        assert_eq!(index.hkcu, vec!["GUID-A", "GUID-B"]);
        assert_eq!(index.hklm, vec!["GUID-A", "GUID-C"]);

        let browser = FolderBrowser::default();

        let folder_hklm = browser.selected_folder(&snapshot);
        assert!(folder_hklm.is_none());

        let mut browser = FolderBrowser::default();
        browser.select_root(RootKey::Hklm);
        browser.select_folder("GUID-A");
        let folder_hklm = browser.selected_folder(&snapshot).unwrap();
        assert_eq!(folder_hklm.canonical_name, Some("System Documents".into()));

        browser.select_root(RootKey::Hkcu);
        browser.select_folder("GUID-A");
        let folder_hkcu = browser.selected_folder(&snapshot).unwrap();
        assert_eq!(folder_hkcu.canonical_name, Some("User Documents".into()));
    }

    #[test]
    fn folder_browser_pagination_is_bounded() {
        let mut browser = FolderBrowser::default();

        assert_eq!(FolderBrowser::page_count(0), 1);
        assert_eq!(browser.page_range(0), 0..0);

        assert_eq!(FolderBrowser::page_count(1), 1);
        assert_eq!(browser.page_range(1), 0..1);

        assert_eq!(FolderBrowser::page_count(25), 1);
        assert_eq!(browser.page_range(25), 0..25);

        assert_eq!(FolderBrowser::page_count(26), 2);
        browser.page = 0;
        assert_eq!(browser.page_range(26), 0..25);
        browser.page = 1;
        assert_eq!(browser.page_range(26), 25..26);

        assert_eq!(FolderBrowser::page_count(51), 3);
        browser.page = 2;
        assert_eq!(browser.page_range(51), 50..51);

        browser.page = usize::MAX;
        let range = browser.page_range(26);
        assert!(range.end <= 26);
        assert!(range.len() <= FOLDERS_PER_PAGE);
    }

    #[test]
    fn folder_browser_navigation_resets_selection() {
        let mut snapshot = snapshot_with_folder_types();

        // Add more entries to create multiple pages
        for i in 3..30 {
            let guid = format!("GUID-{}", i);
            let folder = FolderTypeSnapshot {
                guid: guid.clone(),
                canonical_name: Some(format!("Folder {}", i)),
                top_views: BTreeMap::new(),
                read_errors: vec![],
            };
            snapshot.hklm_folder_types.types.insert(guid, folder);
        }

        let index = FolderTypesIndex::from_snapshot(&snapshot);

        let mut browser = FolderBrowser::default();
        assert_eq!(browser.root, RootKey::Hklm);

        browser.select_folder("GUID-A");
        browser.select_page(1, index.hklm.len());
        assert!(browser.selected_guid.is_none());
        assert_eq!(browser.view_index, 0);

        browser.select_folder("GUID-A");
        browser.view_index = 5;
        browser.select_folder("GUID-C");
        assert_eq!(browser.view_index, 0);

        browser.select_folder("GUID-C");
        browser.view_index = 3;
        browser.select_folder("GUID-C");
        assert_eq!(browser.view_index, 3);

        browser.select_root(RootKey::Hkcu);
        assert_eq!(browser.root, RootKey::Hkcu);
        assert_eq!(browser.page, 0);
        assert!(browser.selected_guid.is_none());
        assert_eq!(browser.view_index, 0);

        browser.select_root(RootKey::Hklm);
        assert_eq!(browser.root, RootKey::Hklm);

        // Selecting the same hive preserves page, selected GUID, and view index.
        browser.select_page(2, index.hklm.len());
        browser.select_folder("GUID-A");
        browser.view_index = 7;
        let page_before = browser.page;
        let guid_before = browser.selected_guid.clone();
        let view_before = browser.view_index;
        browser.select_root(RootKey::Hklm);
        assert_eq!(browser.root, RootKey::Hklm);
        assert_eq!(browser.page, page_before);
        assert_eq!(browser.selected_guid, guid_before);
        assert_eq!(browser.view_index, view_before);
        browser.select_folder("NONEXISTENT");
        assert!(browser.selected_folder(&snapshot).is_none());
    }

    #[test]
    fn folder_view_rows_preserve_captured_values() {
        let view = ViewSnapshot {
            logical_view_mode: Some(0),
            mode: Some(4),
            icon_size: Some(16),
            group_by: Some(String::new()),
            group_ascending: Some(0),
            sort_by_list: Some("Name".into()),
            column_list: None,
            col_info: Some(vec![1, 2, 3]),
            f_flags: Some(0),
            ..Default::default()
        };

        let rows = view_rows(&view);

        assert_eq!(rows[0].0, "LogicalViewMode");
        assert_eq!(rows[0].1, "0");
        assert_eq!(rows[1].0, "Mode");
        assert_eq!(rows[1].1, "4");
        assert_eq!(rows[2].0, "IconSize");
        assert_eq!(rows[2].1, "16");
        assert_eq!(rows[3].0, "GroupBy");
        assert_eq!(rows[3].1, "(empty string)");
        assert_eq!(rows[4].0, "GroupAscending");
        assert_eq!(rows[4].1, "0");
        assert_eq!(rows[5].0, "SortByList");
        assert_eq!(rows[5].1, "Name");
        assert_eq!(rows[6].0, "ColumnList");
        assert_eq!(rows[6].1, "Unavailable");
        assert_eq!(rows[7].0, "ColInfo");
        assert_eq!(rows[7].1, "3 bytes");
        assert_eq!(rows[8].0, "FFlags");
        assert_eq!(rows[8].1, "0");

        let default_view = ViewSnapshot::default();
        let default_rows = view_rows(&default_view);
        for (_, value) in default_rows {
            assert_eq!(value, "Unavailable");
        }
    }

    #[test]
    fn folder_text_previews_are_unicode_safe() {
        let short = "short";
        assert_eq!(preview_text(short), "short");

        let exactly_160: String = "a".repeat(160);
        assert_eq!(preview_text(&exactly_160), exactly_160);

        let over_160: String = "a".repeat(161);
        let preview = preview_text(&over_160);
        assert_eq!(preview.chars().count(), 161);
        assert!(preview.ends_with('…'));

        let korean = "한글 테스트 한국어 문자열";
        let korean_preview = preview_text(korean);
        assert!(!korean_preview.is_empty());

        let emoji = "🎉🚀💻🌟";
        let emoji_preview = preview_text(emoji);
        assert!(!emoji_preview.is_empty());

        for character in ['한', '😀'] {
            let text = character.to_string().repeat(161);

            let preview = preview_text(&text);

            assert_eq!(preview, format!("{}…", character.to_string().repeat(160)));

            assert_eq!(preview.chars().count(), 161);
        }

        assert_eq!(text_or_unavailable(None), "Unavailable");
        assert_eq!(text_or_unavailable(Some("")), "(empty string)");
        assert_eq!(text_or_unavailable(Some("test")), "test");
    }

    #[test]
    fn folder_raw_selection_resets_on_navigation() {
        use winview_core::model::FolderTypeSnapshot;

        let mut snapshot = snapshot_with_folder_types();

        for i in 3..30 {
            let guid = format!("GUID-{}", i);
            let folder = FolderTypeSnapshot {
                guid: guid.clone(),
                canonical_name: Some(format!("Folder {}", i)),
                top_views: BTreeMap::new(),
                read_errors: vec![],
            };
            snapshot.hklm_folder_types.types.insert(guid, folder);
        }

        let index = FolderTypesIndex::from_snapshot(&snapshot);

        let mut browser = FolderBrowser::default();

        assert_eq!(browser.selected_value, None);

        // Selecting the same hive preserves selection.
        browser.selected_value = Some("Mode".into());
        browser.select_root(RootKey::Hklm);
        assert_eq!(browser.selected_value.as_deref(), Some("Mode"));

        // Selecting the same folder preserves selection.
        browser.select_folder("GUID-A");
        browser.selected_value = Some("Mode".into());
        browser.select_folder("GUID-A");
        assert_eq!(browser.selected_value.as_deref(), Some("Mode"));

        // Selecting the same folder page preserves selection.
        browser.selected_value = Some("Mode".into());
        browser.select_page(browser.page, index.hklm.len());
        assert_eq!(browser.selected_value.as_deref(), Some("Mode"));

        // Selecting the same view preserves selection.
        browser.selected_value = Some("Mode".into());
        browser.select_view(browser.view_index);
        assert_eq!(browser.selected_value.as_deref(), Some("Mode"));

        // Changing hive clears selection.
        browser.selected_value = Some("Mode".into());
        browser.select_root(RootKey::Hkcu);
        assert_eq!(browser.selected_value, None);

        // Changing folder page clears selection.
        browser.select_root(RootKey::Hklm);
        browser.selected_value = Some("Mode".into());
        browser.select_page(1, index.hklm.len());
        assert_eq!(browser.selected_value, None);

        // Changing folder clears selection.
        browser.selected_value = Some("Mode".into());
        browser.select_folder("GUID-C");
        assert_eq!(browser.selected_value, None);

        // Changing view clears selection.
        browser.selected_value = Some("Mode".into());
        browser.select_view(browser.view_index + 1);
        assert_eq!(browser.selected_value, None);

        // A new default FolderBrowser clears selection.
        let fresh = FolderBrowser::default();
        assert_eq!(fresh.selected_value, None);
    }

    #[test]
    fn folder_raw_details_preserve_captured_observations() {
        use winview_core::model::{
            DecodingStatus, ObservationSource, RawRegistryValue, ReadError, RegistryValue,
            ValueReadResult,
        };

        fn success(
            name: &str,
            type_id: u32,
            bytes: Vec<u8>,
            decoded: Option<RegistryValue>,
            status: DecodingStatus,
        ) -> ValueReadResult {
            ValueReadResult::Success(RawRegistryValue {
                key_path: r"Software\Test".into(),
                value_name: name.into(),
                root: RootKey::Hkcu,
                raw_type_id: type_id,
                raw_bytes: bytes,
                decoded,
                decoding_status: status,
                source: ObservationSource::HkcuFolderTypes,
            })
        }

        let mut raw_values = BTreeMap::new();
        raw_values.insert(
            "Mode".into(),
            success(
                "Mode",
                4,
                4u32.to_le_bytes().to_vec(),
                Some(RegistryValue::Dword(4)),
                DecodingStatus::Decoded,
            ),
        );
        raw_values.insert(
            "ColInfo".into(),
            success(
                "ColInfo",
                3,
                vec![1],
                None,
                DecodingStatus::Malformed {
                    reason: "Invalid binary length".into(),
                },
            ),
        );
        raw_values.insert(
            "FFlags".into(),
            ValueReadResult::Failure(ReadError::ValueNotFound {
                name: "FFlags".into(),
            }),
        );

        let view = ViewSnapshot {
            logical_view_mode: Some(0),
            mode: Some(4),
            icon_size: Some(16),
            group_by: None,
            group_ascending: Some(0),
            sort_by_list: None,
            column_list: None,
            col_info: Some(vec![1]),
            f_flags: Some(0),
            raw_values,
        };

        let folder = FolderTypeSnapshot {
            guid: "GUID-A".into(),
            canonical_name: Some("User Documents".into()),
            top_views: BTreeMap::from([("VIEW-A".into(), view)]),
            read_errors: vec![],
        };

        let mut snapshot = InspectionSnapshot::default();
        snapshot
            .hkcu_folder_types
            .types
            .insert("GUID-A".into(), folder);

        let before = serde_json::to_string(&snapshot).unwrap();

        let mut browser = FolderBrowser::default();
        browser.select_root(RootKey::Hkcu);
        browser.select_folder("GUID-A");

        let ctx = egui::Context::default();

        // Captured DWORD observation resolves through the raw_values map.
        browser.selected_value = Some("Mode".into());
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            browser.show_details(ui, &snapshot);
        });
        output.textures_delta.clear();
        assert_eq!(browser.selected_value.as_deref(), Some("Mode"));

        // Malformed captured observation still resolves.
        browser.selected_value = Some("ColInfo".into());
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            browser.show_details(ui, &snapshot);
        });
        output.textures_delta.clear();
        assert_eq!(browser.selected_value.as_deref(), Some("ColInfo"));

        // Failed read resolves to the captured failure.
        browser.selected_value = Some("FFlags".into());
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            browser.show_details(ui, &snapshot);
        });
        output.textures_delta.clear();
        assert_eq!(browser.selected_value.as_deref(), Some("FFlags"));

        // A normalized field without a raw observation keeps the selection
        // but does not fabricate an observation.
        browser.selected_value = Some("IconSize".into());
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            browser.show_details(ui, &snapshot);
        });
        output.textures_delta.clear();
        assert_eq!(browser.selected_value.as_deref(), Some("IconSize"));

        let after = serde_json::to_string(&snapshot).unwrap();

        assert_eq!(before, after);
    }
}
