// winview-app/src/gui/folder_types.rs

use eframe::egui;

use winview_core::model::{
    FolderTypeSnapshot, FolderTypesSnapshot, InspectionSnapshot, ViewSnapshot,
};

use winview_core::RootKey;

use super::diagnostics::ErrorPager;
use super::style::helpers;
use super::style::{
    StatusKind, FOLDER_LIST_MAX_WIDTH, FOLDER_LIST_MIN_WIDTH, FOLDER_LIST_WIDTH,
    WIDE_WINDOW_THRESHOLD,
};

const FOLDERS_PER_PAGE: usize = 25;
const TEXT_PREVIEW_CHARS: usize = 160;

#[derive(Default, Clone)]
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
    hive_error_pager: ErrorPager,
    folder_error_pager: ErrorPager,
    search_query: String,
    filtered_guids: Option<Vec<String>>,
}

impl Default for FolderBrowser {
    fn default() -> Self {
        Self {
            root: RootKey::Hklm,
            page: 0,
            selected_guid: None,
            view_index: 0,
            selected_value: None,
            hive_error_pager: ErrorPager::default(),
            folder_error_pager: ErrorPager::default(),
            search_query: String::new(),
            filtered_guids: None,
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
            self.search_query.clear();
            self.filtered_guids = None;
            self.hive_error_pager.reset();
            self.folder_error_pager.reset();
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
            self.folder_error_pager.reset();
        }
    }

    fn select_folder(&mut self, guid: &str) {
        if self.selected_guid.as_deref() != Some(guid) {
            self.selected_guid = Some(guid.to_string());

            self.view_index = 0;
            self.selected_value = None;
            self.folder_error_pager.reset();
        }
    }

    fn select_view(&mut self, index: usize) {
        if self.view_index != index {
            self.view_index = index;
            self.selected_value = None;
        }
    }

    fn filtered_keys(&self, index: &FolderTypesIndex) -> Vec<String> {
        if let Some(filtered) = &self.filtered_guids {
            filtered.clone()
        } else {
            index.keys(self.root).to_vec()
        }
    }

    fn update_search(
        &mut self,
        query: String,
        index: &FolderTypesIndex,
        snapshot: &InspectionSnapshot,
    ) {
        let trimmed = query.trim().to_lowercase();

        if trimmed.is_empty() {
            self.search_query.clear();
            self.filtered_guids = None;
        } else {
            self.search_query = trimmed.clone();
            let folder_types = folder_types_for_root(snapshot, self.root);
            let keys = index.keys(self.root);
            let filtered: Vec<String> = keys
                .iter()
                .filter(|guid| {
                    let guid_lower = guid.to_lowercase();
                    let matches_guid = guid_lower.contains(&trimmed);
                    let matches_name = folder_types
                        .types
                        .get(*guid)
                        .and_then(|f| f.canonical_name.as_deref())
                        .map(|name| name.to_lowercase().contains(&trimmed))
                        .unwrap_or(false);
                    matches_guid || matches_name
                })
                .cloned()
                .collect();
            self.filtered_guids = Some(filtered);
        }

        self.page = 0;
        self.selected_guid = None;
        self.view_index = 0;
        self.selected_value = None;
        self.folder_error_pager.reset();
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
        let colors = super::style::ThemeColors::from_style(&ui.style());
        let available_width = ui.available_width();
        let is_wide = available_width >= WIDE_WINDOW_THRESHOLD;

        helpers::section_heading(ui, "Folder types");

        // Hive selector
        ui.horizontal(|ui| {
            let hklm_selected = self.root == RootKey::Hklm;
            if helpers::nav_item_small(ui, hklm_selected, "System defaults (HKLM)").clicked() {
                self.select_root(RootKey::Hklm);
            }

            let hkcu_selected = self.root == RootKey::Hkcu;
            if helpers::nav_item_small(ui, hkcu_selected, "User overrides (HKCU)").clicked() {
                self.select_root(RootKey::Hkcu);
            }
        });

        ui.add_space(8.0);

        // Search input
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Search:")
                    .size(13.0)
                    .color(colors.text_secondary),
            );
            ui.add_space(8.0);
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.search_query)
                    .hint_text("Filter by canonical name or GUID...")
                    .desired_width(300.0),
            );
            if response.changed() {
                self.update_search(self.search_query.clone(), index, snapshot);
            }
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.search_query.clear();
                self.update_search(String::new(), index, snapshot);
                response.request_focus();
            }
        });

        ui.add_space(8.0);

        let folder_types = folder_types_for_root(snapshot, self.root);
        let keys = self.filtered_keys(index);

        if self.filtered_guids.is_some() {
            ui.label(
                egui::RichText::new(format!(
                    "{} matching folder-type records (of {} total)",
                    keys.len(),
                    index.keys(self.root).len()
                ))
                .size(12.0)
                .color(colors.text_secondary),
            );
        } else {
            ui.label(
                egui::RichText::new(format!("{} captured folder-type records", keys.len()))
                    .size(12.0)
                    .color(colors.text_secondary),
            );
        }

        ui.label(
            egui::RichText::new(format!(
                "Hive-level inspection errors: {}",
                folder_types.read_errors.len(),
            ))
            .size(12.0)
            .color(colors.text_secondary),
        );

        ui.small(
            egui::RichText::new(
                "These are captured registry records, \
                 not necessarily effective Explorer settings.",
            )
            .color(colors.text_secondary),
        );

        self.hive_error_pager.show(
            ui,
            "FolderTypes hive inspection errors",
            &folder_types.read_errors,
        );

        if keys.is_empty() {
            if self.filtered_guids.is_some() {
                ui.add_space(16.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new("No matching folder-type records found.")
                            .size(14.0)
                            .color(colors.text_secondary),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Try a different search term or clear the filter.")
                            .size(12.0)
                            .color(colors.text_secondary),
                    );
                });
            } else {
                ui.label("No folder-type records were captured in this hive.");
            }

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
                ui.ctx().request_repaint();
            }

            if ui
                .add_enabled(self.page + 1 < pages, egui::Button::new("Next"))
                .clicked()
            {
                self.select_page(self.page + 1, keys.len());
                ui.ctx().request_repaint();
            }

            ui.add_space(16.0);
            ui.label(
                egui::RichText::new(format!("Page {} of {}", self.page + 1, pages))
                    .size(13.0)
                    .color(colors.text_secondary),
            );
        });

        ui.add_space(8.0);

        let range = self.page_range(keys.len());

        if is_wide {
            // Master-detail layout
            let list_width = FOLDER_LIST_WIDTH.clamp(FOLDER_LIST_MIN_WIDTH, FOLDER_LIST_MAX_WIDTH);

            egui::Panel::left("folder_list_panel")
                .exact_size(list_width)
                .resizable(false)
                .show_separator_line(true)
                .frame(egui::Frame::new().fill(egui::Color32::TRANSPARENT))
                .show(ui, |ui| {
                    self.show_folder_list(ui, &keys[range], folder_types);
                });

            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(egui::Color32::TRANSPARENT))
                .show(ui, |ui| {
                    self.show_details(ui, snapshot);
                });
        } else {
            // Stacked layout
            egui::ScrollArea::vertical()
                .id_salt("folder_list_scroll")
                .max_height(300.0)
                .show(ui, |ui| {
                    self.show_folder_list(ui, &keys[range], folder_types);
                });

            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);

            self.show_details(ui, snapshot);
        }

        if let Some(folder) = self.selected_folder(snapshot) {
            self.folder_error_pager.show(
                ui,
                "Selected folder inspection errors",
                &folder.read_errors,
            );
        } else {
            self.folder_error_pager.reset();
        }
    }

    fn show_folder_list(
        &mut self,
        ui: &mut egui::Ui,
        guids: &[String],
        folder_types: &FolderTypesSnapshot,
    ) {
        let colors = super::style::ThemeColors::from_style(&ui.style());

        for guid in guids {
            let Some(folder) = folder_types.types.get(guid) else {
                continue;
            };

            let name = text_or_unavailable(folder.canonical_name.as_deref());

            ui.push_id(guid.as_str(), |ui| {
                let response = ui.allocate_response(
                    egui::vec2(ui.available_width(), 44.0),
                    egui::Sense::click(),
                );

                let selected = self.selected_guid.as_deref() == Some(guid.as_str());
                let bg = if selected {
                    colors.accent.gamma_multiply(0.12)
                } else if response.hovered() {
                    colors.accent.gamma_multiply(0.06)
                } else {
                    egui::Color32::TRANSPARENT
                };

                if bg != egui::Color32::TRANSPARENT {
                    ui.painter().rect_filled(response.rect, 6.0, bg);
                }

                if selected {
                    ui.painter().rect_stroke(
                        response.rect,
                        6.0,
                        egui::Stroke::new(2.0, colors.accent),
                        egui::StrokeKind::Outside,
                    );
                }

                let text_pos = response.rect.left_top() + egui::vec2(12.0, 8.0);
                ui.painter().text(
                    text_pos,
                    egui::Align2::LEFT_TOP,
                    &name,
                    egui::FontId::new(14.0, egui::FontFamily::Proportional),
                    colors.text,
                );

                let guid_pos = response.rect.left_top() + egui::vec2(12.0, 26.0);
                ui.painter().text(
                    guid_pos,
                    egui::Align2::LEFT_TOP,
                    guid.as_str(),
                    egui::FontId::new(11.0, egui::FontFamily::Monospace),
                    colors.text_secondary,
                );

                if response.clicked() {
                    self.select_folder(guid);
                }
            });

            ui.add_space(4.0);
        }
    }

    fn show_details(&mut self, ui: &mut egui::Ui, snapshot: &InspectionSnapshot) {
        let colors = super::style::ThemeColors::from_style(&ui.style());

        let Some(folder) = self.selected_folder(snapshot) else {
            self.selected_value = None;

            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(
                    egui::RichText::new("Select a folder type to inspect its captured settings.")
                        .size(14.0)
                        .color(colors.text_secondary),
                );
            });

            return;
        };

        // Folder header
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(text_or_unavailable(folder.canonical_name.as_deref()))
                        .size(18.0)
                        .strong()
                        .color(colors.text),
                );
                ui.label(
                    egui::RichText::new(format!("GUID: {}", folder.guid))
                        .size(12.0)
                        .color(colors.text_secondary),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                helpers::status_badge(
                    ui,
                    &format!("Folder errors: {}", folder.read_errors.len()),
                    if folder.read_errors.is_empty() {
                        StatusKind::Success
                    } else {
                        StatusKind::Warning
                    },
                );
            });
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        if folder.top_views.is_empty() {
            self.selected_value = None;

            ui.label(
                egui::RichText::new("No TopViews records were captured.")
                    .size(13.0)
                    .color(colors.text_secondary),
            );

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
                    ui.ctx().request_repaint();
                }

                if ui
                    .add_enabled(self.view_index + 1 < count, egui::Button::new("Next view"))
                    .clicked()
                {
                    self.select_view(self.view_index + 1);
                    ui.ctx().request_repaint();
                }

                ui.add_space(16.0);
                ui.label(
                    egui::RichText::new(format!("View {} of {}", self.view_index + 1, count))
                        .size(13.0)
                        .color(colors.text_secondary),
                );
            });

            ui.add_space(8.0);
        }

        let Some((guid, view)) = folder.top_views.iter().nth(self.view_index) else {
            self.selected_value = None;

            return;
        };

        ui.label(
            egui::RichText::new(format!("View GUID: {guid}"))
                .size(12.0)
                .color(colors.text_secondary),
        );

        ui.add_space(8.0);

        // View settings table
        egui::Grid::new("folder_view_details")
            .num_columns(2)
            .striped(true)
            .min_col_width(180.0)
            .show(ui, |ui| {
                for (label, value) in view_rows(view) {
                    let selected = self.selected_value.as_deref() == Some(label);

                    if ui
                        .selectable_label(
                            selected,
                            egui::RichText::new(label).size(13.0).color(colors.text),
                        )
                        .clicked()
                    {
                        self.selected_value = Some(label.to_string());
                    }

                    ui.label(egui::RichText::new(value).size(13.0).color(colors.text));
                    ui.end_row();
                }
            });

        ui.small(
            egui::RichText::new(
                "Unavailable fields may be missing, \
                 unreadable, omitted, or undecodable.",
            )
            .color(colors.text_secondary),
        );

        if let Some(name) = self.selected_value.as_deref() {
            match view.raw_values.get(name) {
                Some(observation) => {
                    super::value_details::show_value_details(ui, name, observation);
                }
                None => {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                    helpers::section_heading(ui, "Value details");
                    ui.label(name);
                    ui.label(
                        egui::RichText::new("No raw observation was captured for this field.")
                            .size(13.0)
                            .color(colors.text_secondary),
                    );
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
            "Logical view mode (LogicalViewMode)",
            number_or_unavailable(view.logical_view_mode),
        ),
        ("Mode", number_or_unavailable(view.mode)),
        ("Icon size", number_or_unavailable(view.icon_size)),
        ("Group by", text_or_unavailable(view.group_by.as_deref())),
        (
            "Group ascending",
            number_or_unavailable(view.group_ascending),
        ),
        (
            "Sort by list",
            text_or_unavailable(view.sort_by_list.as_deref()),
        ),
        (
            "Column list",
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

        assert_eq!(rows[0].0, "Logical view mode (LogicalViewMode)");
        assert_eq!(rows[0].1, "0");
        assert_eq!(rows[1].0, "Mode");
        assert_eq!(rows[1].1, "4");
        assert_eq!(rows[2].0, "Icon size");
        assert_eq!(rows[2].1, "16");
        assert_eq!(rows[3].0, "Group by");
        assert_eq!(rows[3].1, "(empty string)");
        assert_eq!(rows[4].0, "Group ascending");
        assert_eq!(rows[4].1, "0");
        assert_eq!(rows[5].0, "Sort by list");
        assert_eq!(rows[5].1, "Name");
        assert_eq!(rows[6].0, "Column list");
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

    #[test]
    fn folder_diagnostics_preserve_scope_and_navigation() {
        use winview_core::inspection::count_snapshot_read_errors;
        use winview_core::model::InspectionError;

        fn error(path: &str, message: &str) -> InspectionError {
            InspectionError {
                key_path: path.into(),
                root: RootKey::Hklm,
                operation: "open_key".into(),
                error: message.into(),
            }
        }

        let mut snapshot = InspectionSnapshot::default();

        // HKLM hive-level errors (enough for multiple diagnostic pages).
        snapshot.hklm_folder_types.read_errors = (0..30)
            .map(|i| error("HKLM-hive", &format!("hive error {i}")))
            .collect();

        // HKCU hive is empty of records but still contains recorded errors.
        snapshot.hkcu_folder_types.read_errors = (0..3)
            .map(|i| InspectionError {
                key_path: "HKCU-hive".into(),
                root: RootKey::Hkcu,
                operation: "open_key".into(),
                error: format!("hkcu hive error {i}"),
            })
            .collect();

        // Selected folder with recorded errors and a TopViews record.
        let folder_a = FolderTypeSnapshot {
            guid: "GUID-A".into(),
            canonical_name: Some("Folder A".into()),
            top_views: BTreeMap::from([("VIEW-A".into(), ViewSnapshot::default())]),
            read_errors: (0..30)
                .map(|i| error("GUID-A", &format!("folder A error {i}")))
                .collect(),
        };

        // Second folder with different recorded errors.
        let folder_b = FolderTypeSnapshot {
            guid: "GUID-B".into(),
            canonical_name: Some("Folder B".into()),
            top_views: BTreeMap::new(),
            read_errors: (0..2)
                .map(|i| error("GUID-B", &format!("folder B error {i}")))
                .collect(),
        };

        snapshot
            .hklm_folder_types
            .types
            .insert("GUID-A".into(), folder_a);
        snapshot
            .hklm_folder_types
            .types
            .insert("GUID-B".into(), folder_b);

        let before = count_snapshot_read_errors(&snapshot);
        let before_json = serde_json::to_string(&snapshot).unwrap();

        let mut browser = FolderBrowser::default();

        // A fresh browser has both pagers on the first page.
        assert_eq!(browser.hive_error_pager.page_range(30).start, 0);
        assert_eq!(browser.folder_error_pager.page_range(30).start, 0);

        browser.select_root(RootKey::Hklm);
        browser.select_folder("GUID-A");

        // Move both pagers to the second page.
        browser.hive_error_pager.select_page(1, 30);
        browser.folder_error_pager.select_page(1, 30);
        assert_eq!(browser.hive_error_pager.page_range(30).start, 25);
        assert_eq!(browser.folder_error_pager.page_range(30).start, 25);

        // Selecting the same hive preserves both pager states.
        browser.select_root(RootKey::Hklm);
        assert_eq!(browser.hive_error_pager.page_range(30).start, 25);
        assert_eq!(browser.folder_error_pager.page_range(30).start, 25);

        // Selecting the same folder preserves both pager states.
        browser.select_folder("GUID-A");
        assert_eq!(browser.hive_error_pager.page_range(30).start, 25);
        assert_eq!(browser.folder_error_pager.page_range(30).start, 25);

        // Changing the folder resets only the folder pager.
        browser.select_folder("GUID-B");
        assert_eq!(browser.hive_error_pager.page_range(30).start, 25);
        assert_eq!(browser.folder_error_pager.page_range(30).start, 0);

        // Changing the folder page resets only the folder pager.
        browser.select_folder("GUID-A");
        browser.folder_error_pager.select_page(1, 30);
        browser.select_page(1, 30);
        assert_eq!(browser.hive_error_pager.page_range(30).start, 25);
        assert_eq!(browser.folder_error_pager.page_range(30).start, 0);

        // Changing the view preserves both pagers.
        browser.select_folder("GUID-A");
        browser.hive_error_pager.select_page(1, 30);
        browser.folder_error_pager.select_page(1, 30);
        browser.select_view(1);
        assert_eq!(browser.hive_error_pager.page_range(30).start, 25);
        assert_eq!(browser.folder_error_pager.page_range(30).start, 25);

        // Changing the hive resets both pagers.
        browser.select_root(RootKey::Hkcu);
        assert_eq!(browser.hive_error_pager.page_range(30).start, 0);
        assert_eq!(browser.folder_error_pager.page_range(30).start, 0);

        // A fresh FolderBrowser resets both pagers.
        let mut fresh = FolderBrowser::default();
        assert_eq!(fresh.hive_error_pager.page_range(30).start, 0);
        assert_eq!(fresh.folder_error_pager.page_range(30).start, 0);

        // Exercise the real renderer with an empty hive that has recorded errors.
        let ctx = egui::Context::default();

        browser.select_root(RootKey::Hkcu);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            browser.show(ui, &snapshot, &FolderTypesIndex::from_snapshot(&snapshot));
        });
        output.textures_delta.clear();

        // Exercise the renderer with a selected folder that has no TopViews records.
        browser.select_root(RootKey::Hklm);
        browser.select_folder("GUID-B");
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            browser.show(ui, &snapshot, &FolderTypesIndex::from_snapshot(&snapshot));
        });
        output.textures_delta.clear();

        let after = count_snapshot_read_errors(&snapshot);
        let after_json = serde_json::to_string(&snapshot).unwrap();

        assert_eq!(before, after);
        assert_eq!(before_json, after_json);
    }

    #[test]
    fn folder_search_filters_by_guid_and_name() {
        let mut snapshot = snapshot_with_folder_types();
        let index = FolderTypesIndex::from_snapshot(&snapshot);

        let mut browser = FolderBrowser::default();

        // Search by GUID substring
        browser.update_search("GUID-A".to_string(), &index, &snapshot);
        assert!(browser.filtered_guids.is_some());
        let filtered = browser.filtered_guids.as_ref().unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0], "GUID-A");

        // Search by canonical name substring
        browser.update_search("Documents".to_string(), &index, &snapshot);
        assert!(browser.filtered_guids.is_some());
        let filtered = browser.filtered_guids.as_ref().unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0], "GUID-A");

        // Case insensitive
        browser.update_search("documents".to_string(), &index, &snapshot);
        let filtered = browser.filtered_guids.as_ref().unwrap();
        assert_eq!(filtered.len(), 1);

        // Empty query clears filter
        browser.update_search("".to_string(), &index, &snapshot);
        assert!(browser.filtered_guids.is_none());

        // Whitespace only clears filter
        browser.update_search("   ".to_string(), &index, &snapshot);
        assert!(browser.filtered_guids.is_none());
    }

    #[test]
    fn folder_search_resets_selection_and_pagination() {
        let mut snapshot = snapshot_with_folder_types();
        let index = FolderTypesIndex::from_snapshot(&snapshot);

        let mut browser = FolderBrowser::default();
        browser.select_root(RootKey::Hklm);
        browser.select_folder("GUID-A");
        browser.page = 1;
        browser.selected_value = Some("Mode".into());

        browser.update_search("GUID-A".to_string(), &index, &snapshot);

        assert_eq!(browser.page, 0);
        assert!(browser.selected_guid.is_none());
        assert_eq!(browser.view_index, 0);
        assert_eq!(browser.selected_value, None);
    }

    #[test]
    fn folder_search_preserves_duplicate_guids_across_hives() {
        // GUID-A exists in both HKCU and HKLM with different canonical names
        let mut snapshot = snapshot_with_folder_types();
        let index = FolderTypesIndex::from_snapshot(&snapshot);

        let mut browser = FolderBrowser::default();
        browser.select_root(RootKey::Hklm);
        browser.update_search("GUID-A".to_string(), &index, &snapshot);
        let filtered = browser.filtered_guids.as_ref().unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0], "GUID-A");

        browser.select_root(RootKey::Hkcu);
        browser.update_search("GUID-A".to_string(), &index, &snapshot);
        let filtered = browser.filtered_guids.as_ref().unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0], "GUID-A");
    }

    #[test]
    fn folder_search_pagination_works() {
        let mut snapshot = snapshot_with_folder_types();

        // Add 30 more entries
        for i in 3..33 {
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
        browser.select_root(RootKey::Hklm);

        // Search matching all
        browser.update_search("GUID".to_string(), &index, &snapshot);
        assert!(browser.filtered_guids.is_some());
        let filtered: Vec<String> = browser.filtered_guids.as_ref().unwrap().clone();
        assert_eq!(filtered.len(), 32); // 2 original + 30 new

        assert_eq!(FolderBrowser::page_count(32), 2);
        assert_eq!(browser.page_range(filtered.len()), 0..25);

        browser.select_page(1, filtered.len());
        assert_eq!(browser.page_range(filtered.len()), 25..32);
    }
}
