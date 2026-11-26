// winview-app/src/gui/app.rs

use std::sync::Arc;
use std::time::Duration;

use eframe::egui;

use winview_core::model::InspectionStatus;

use winview_core::RegistryReader;

use super::inspection::InspectionState;

use super::advanced::AdvancedBrowser;
use super::diagnostics::ErrorPager;
use super::export::ExportBrowser;
use super::folder_types::FolderBrowser;
use super::overview::show_overview;
use super::registry_browser::RegistryBrowser;
use super::style::{helpers, StatusKind, ThemeColors, SIDEBAR_WIDTH, WIDE_WINDOW_THRESHOLD};

#[derive(Default, PartialEq, Eq, Copy, Clone)]
enum InspectorPage {
    #[default]
    Overview,
    FolderTypes,
    Advanced,
    RegistryDomains,
}

#[derive(Default)]
pub struct WinViewApp {
    inspection: InspectionState,
    page: InspectorPage,
    folder_browser: FolderBrowser,
    registry_browser: RegistryBrowser,
    advanced_browser: AdvancedBrowser,
    global_error_pager: ErrorPager,
    export_browser: ExportBrowser,
    show_export_panel: bool,
    style_configured: bool,
}

impl eframe::App for WinViewApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.style_configured {
            super::style::configure_style(ui.ctx());
            self.style_configured = true;
        }

        self.inspection.poll();
        self.export_browser.poll();

        let available_width = ui.available_width();
        let is_wide = available_width >= WIDE_WINDOW_THRESHOLD;
        let sidebar_w = if is_wide { SIDEBAR_WIDTH } else { 0.0 };

        if is_wide {
            egui::Panel::left("navigation_sidebar")
                .exact_size(sidebar_w)
                .resizable(false)
                .show_separator_line(false)
                .frame(egui::Frame::new().fill(egui::Color32::TRANSPARENT))
                .show(ui, |ui| {
                    self.show_sidebar(ui);
                });
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(egui::Color32::TRANSPARENT))
            .show(ui, |ui| {
                self.show_main_content(ui, is_wide);
            });

        if self.export_browser.is_running() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }
}

impl WinViewApp {
    fn show_sidebar(&mut self, ui: &mut egui::Ui) {
        let colors = ThemeColors::from_style(&ui.style());

        ui.vertical(|ui| {
            ui.add_space(16.0);

            // Application title
            ui.label(
                egui::RichText::new("WinViewRs")
                    .size(22.0)
                    .strong()
                    .color(colors.text),
            );

            ui.add_space(4.0);

            // Read-only mode indicator
            helpers::status_badge(ui, "Read-only mode", StatusKind::Info);

            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);

            // Navigation items
            let nav_items = [
                (InspectorPage::Overview, "Overview"),
                (InspectorPage::FolderTypes, "Folder types"),
                (InspectorPage::Advanced, "Advanced settings"),
                (InspectorPage::RegistryDomains, "Registry browser"),
            ];

            for (page, label) in nav_items {
                let selected = self.page == page;
                if helpers::nav_item(ui, selected, label).clicked() {
                    self.page = page;
                }
                ui.add_space(4.0);
            }
        });
    }

    fn show_main_content(&mut self, ui: &mut egui::Ui, is_wide: bool) {
        if !is_wide {
            self.show_compact_navigation(ui);
        }

        // Extract the inspection state kind first to avoid borrow issues
        let state_kind = self.inspection.kind();

        match state_kind {
            super::inspection::InspectionStateKind::Idle => {
                self.show_idle_state(ui);
            }
            super::inspection::InspectionStateKind::Loading => {
                self.show_loading_state(ui);
            }
            super::inspection::InspectionStateKind::Finished => {
                // Use a block to limit the borrow scope
                let result = match &self.inspection {
                    super::inspection::InspectionState::Finished(r) => r.clone(),
                    _ => unreachable!(),
                };
                self.show_inspection_results(ui, &result);
            }
            super::inspection::InspectionStateKind::Error => {
                let error = match &self.inspection {
                    super::inspection::InspectionState::Error(e) => e.clone(),
                    _ => unreachable!(),
                };
                self.show_error_state(ui, &error);
            }
        }
    }

    fn show_compact_navigation(&mut self, ui: &mut egui::Ui) {
        let colors = ThemeColors::from_style(&ui.style());

        ui.horizontal_wrapped(|ui| {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("WinViewRs")
                    .size(18.0)
                    .strong()
                    .color(colors.text),
            );
            ui.add_space(8.0);
            helpers::status_badge(ui, "Read-only", StatusKind::Info);
        });

        ui.add_space(8.0);

        ui.horizontal_wrapped(|ui| {
            let nav_items = [
                (InspectorPage::Overview, "Overview"),
                (InspectorPage::FolderTypes, "Folder types"),
                (InspectorPage::Advanced, "Advanced"),
                (InspectorPage::RegistryDomains, "Registry"),
            ];

            for (page, label) in nav_items {
                let selected = self.page == page;
                if helpers::nav_item_small(ui, selected, label).clicked() {
                    self.page = page;
                }
                ui.add_space(4.0);
            }
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);
    }

    fn show_idle_state(&mut self, ui: &mut egui::Ui) {
        let colors = ThemeColors::from_style(&ui.style());

        ui.vertical_centered(|ui| {
            ui.add_space(60.0);
            ui.label(
                egui::RichText::new(
                    "Run an inspection to read your current Explorer configuration.",
                )
                .size(15.0)
                .color(colors.text_secondary),
            );
            ui.add_space(24.0);
            if ui
                .add_enabled(true, egui::Button::new("Run inspection"))
                .clicked()
            {
                self.start_inspection();
            }
        });
    }

    fn show_loading_state(&mut self, ui: &mut egui::Ui) {
        let colors = ThemeColors::from_style(&ui.style());

        ui.vertical_centered(|ui| {
            ui.add_space(60.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new("Reading registry configuration...")
                        .size(15.0)
                        .color(colors.text),
                );
            });
        });
        ui.ctx().request_repaint_after(Duration::from_millis(50));
    }

    fn show_error_state(&mut self, ui: &mut egui::Ui, error: &str) {
        let colors = ThemeColors::from_style(&ui.style());

        ui.vertical_centered(|ui| {
            ui.add_space(60.0);
            ui.heading(egui::RichText::new("Inspection failed").color(colors.error));
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new(error)
                    .size(14.0)
                    .color(colors.text_secondary),
            );
            ui.add_space(24.0);
            if ui
                .add_enabled(true, egui::Button::new("Try again"))
                .clicked()
            {
                self.start_inspection();
            }
        });
    }

    fn show_inspection_results(
        &mut self,
        ui: &mut egui::Ui,
        result: &super::inspection::CapturedInspection,
    ) {
        let snapshot = &result.snapshot;
        let overview = &result.overview;

        // Status bar
        self.show_status_bar(ui, snapshot, result.error_count);

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        // Toolbar actions
        self.show_toolbar(ui, snapshot);

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        // Export panel
        if self.show_export_panel {
            self.show_export_panel(ui, &result.snapshot);
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);
        }

        // Page content
        egui::ScrollArea::vertical()
            .id_salt("main_content_scroll")
            .show(ui, |ui| match self.page {
                InspectorPage::Overview => {
                    show_overview(ui, overview);
                    self.global_error_pager
                        .show(ui, "Global inspection errors", &snapshot.errors);
                }
                InspectorPage::FolderTypes => {
                    self.folder_browser.show(ui, snapshot, &result.folder_index);
                }
                InspectorPage::Advanced => {
                    self.advanced_browser.show(ui, &snapshot.advanced);
                }
                InspectorPage::RegistryDomains => {
                    self.registry_browser.show(ui, snapshot);
                }
            });
    }

    fn show_status_bar(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &winview_core::model::InspectionSnapshot,
        error_count: usize,
    ) {
        let colors = ThemeColors::from_style(&ui.style());

        ui.horizontal(|ui| {
            // Status
            match &snapshot.status {
                InspectionStatus::Complete => {
                    helpers::status_badge(ui, "Inspection complete", StatusKind::Success);
                }
                InspectionStatus::Partial { reason } => {
                    helpers::status_badge(ui, "Inspection partial", StatusKind::Warning);
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(reason)
                            .size(12.0)
                            .color(colors.text_secondary),
                    );
                }
                InspectionStatus::Failed { error } => {
                    helpers::status_badge(ui, "Inspection failed", StatusKind::Error);
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(error)
                            .size(12.0)
                            .color(colors.text_secondary),
                    );
                }
            }

            ui.add_space(16.0);

            // Error count
            let error_kind = if error_count > 0 {
                StatusKind::Warning
            } else {
                StatusKind::Success
            };
            helpers::status_badge(ui, &format!("Recorded errors: {}", error_count), error_kind);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Capture timestamp
                if let Some(product_name) = &snapshot.metadata.windows_product_name {
                    ui.label(
                        egui::RichText::new(format!("Registry product name: {}", product_name))
                            .size(11.0)
                            .color(colors.text_secondary),
                    );
                }
            });
        });
    }

    fn show_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        _snapshot: &winview_core::model::InspectionSnapshot,
    ) {
        let loading = self.inspection.is_loading();
        let exporting = self.export_browser.is_running();
        let disabled = loading || exporting;

        ui.horizontal_wrapped(|ui| {
            // Run/Refresh inspection button
            let button_label = if loading {
                "Inspecting..."
            } else if matches!(&self.inspection, InspectionState::Idle) {
                "Run inspection"
            } else {
                "Refresh inspection"
            };

            if ui
                .add_enabled(!disabled, egui::Button::new(button_label))
                .clicked()
            {
                self.start_inspection();
            }

            ui.add_space(12.0);

            // Export button
            if ui
                .add_enabled(!disabled, egui::Button::new("Export snapshot"))
                .clicked()
            {
                self.show_export_panel = !self.show_export_panel;
                if self.show_export_panel {
                    self.export_browser.reset();
                }
            }
        });
    }

    fn show_export_panel(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &Arc<winview_core::model::InspectionSnapshot>,
    ) {
        let colors = ThemeColors::from_style(&ui.style());

        egui::Frame::new()
            .fill(colors.panel_background)
            .stroke(egui::Stroke::new(1.0, colors.border))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Export captured snapshot");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").clicked() {
                            self.show_export_panel = false;
                        }
                    });
                });

                ui.add_space(8.0);

                ui.label(
                    egui::RichText::new(
                        "Exports contain the complete captured snapshot, including raw values and recorded diagnostics."
                    )
                    .size(12.0)
                    .color(colors.text_secondary),
                );

                ui.add_space(4.0);

                ui.label(
                    egui::RichText::new(
                        "Snapshots may contain personal paths and system-specific information."
                    )
                    .size(12.0)
                    .color(colors.text_secondary),
                );

                ui.add_space(8.0);

                let running = self.export_browser.is_running();
                ui.add_enabled(
                    !running,
                    egui::TextEdit::singleline(self.export_browser.destination_mut())
                        .desired_width(f32::INFINITY)
                        .hint_text("C:\\Users\\YourName\\Documents\\winview-snapshot.json"),
                );

                ui.add_space(8.0);

                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(!running, egui::Button::new("Export JSON"))
                        .clicked()
                    {
                        self.export_browser.start(Arc::clone(snapshot));
                    }

                    if running {
                        ui.spinner();
                        ui.label("Exporting captured snapshot...");
                    }
                });

                ui.add_space(8.0);

                ui.small(
                    egui::RichText::new("Existing files will not be overwritten.")
                        .color(colors.text_secondary),
                );
                ui.small(
                    egui::RichText::new("The exported snapshot is not a restorable registry backup.")
                        .color(colors.text_secondary),
                );
                ui.small(
                    egui::RichText::new("Keep the application open until the export completes.")
                        .color(colors.text_secondary),
                );
                ui.small(
                    egui::RichText::new("A failed or interrupted write may leave an incomplete new file. Export is not atomic.")
                        .color(colors.text_secondary),
                );

                if let Some(result) = self.export_browser.result() {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                    match result {
                        Ok(path) => {
                            ui.label(
                                egui::RichText::new(format!("Export complete: {}", path.display()))
                                    .color(colors.success)
                                    .strong(),
                            );
                        }
                        Err(error) => {
                            ui.label(
                                egui::RichText::new(format!("Export failed: {}", error))
                                    .color(colors.error)
                                    .strong(),
                            );
                        }
                    }
                }
            });
    }

    fn start_inspection(&mut self) {
        let _ = self.inspection.start(|| {
            let reader = winview_registry::WindowsRegistryReader;
            reader.inspect_all().map_err(|error| error.to_string())
        });

        self.folder_browser = FolderBrowser::default();
        self.registry_browser = RegistryBrowser::default();
        self.advanced_browser = AdvancedBrowser::default();
        self.global_error_pager = ErrorPager::default();
        self.export_browser.reset();
        self.show_export_panel = false;
        self.page = InspectorPage::Overview;
    }
}
