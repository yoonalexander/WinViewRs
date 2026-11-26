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

#[derive(Default, PartialEq, Eq)]
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
}

impl eframe::App for WinViewApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.inspection.poll();
        self.export_browser.poll();

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("WinViewRs");

            ui.label("Windows Explorer Configuration Inspector");

            ui.separator();

            ui.label("Read-only mode");

            ui.add_space(8.0);

            let loading = self.inspection.is_loading();

            let exporting = self.export_browser.is_running();

            let button_label = if loading {
                "Inspecting..."
            } else if matches!(&self.inspection, InspectionState::Idle) {
                "Run inspection"
            } else {
                "Refresh inspection"
            };

            if ui
                .add_enabled(!loading && !exporting, egui::Button::new(button_label))
                .clicked()
                && self.inspection.start(|| {
                    let reader = winview_registry::WindowsRegistryReader;

                    reader.inspect_all().map_err(|error| error.to_string())
                })
            {
                self.folder_browser = FolderBrowser::default();

                self.registry_browser = RegistryBrowser::default();

                self.advanced_browser = AdvancedBrowser::default();

                self.global_error_pager = ErrorPager::default();

                self.export_browser.reset();

                self.page = InspectorPage::Overview;
            }

            ui.add_space(8.0);

            match &self.inspection {
                InspectionState::Idle => {
                    ui.label("Run an inspection to read your current Explorer configuration.");
                }

                InspectionState::Loading(_) => {
                    ui.horizontal(|ui| {
                        ui.spinner();

                        ui.label("Reading registry configuration...");
                    });

                    ui.ctx().request_repaint_after(Duration::from_millis(50));
                }

                InspectionState::Finished(result) => {
                    match &result.snapshot.status {
                        InspectionStatus::Complete => {
                            ui.heading("Inspection complete");
                        }

                        InspectionStatus::Partial { reason } => {
                            ui.heading("Inspection partial");

                            ui.label(reason);
                        }

                        InspectionStatus::Failed { error } => {
                            ui.heading("Inspection failed");

                            ui.label(error);
                        }
                    }

                    ui.label(format!(
                        "Recorded inspection errors: {}",
                        result.error_count
                    ));

                    self.export_browser.show(ui, &result.snapshot);

                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.page, InspectorPage::Overview, "Overview");

                        ui.selectable_value(
                            &mut self.page,
                            InspectorPage::FolderTypes,
                            "FolderTypes",
                        );

                        ui.selectable_value(&mut self.page, InspectorPage::Advanced, "Advanced");

                        ui.selectable_value(
                            &mut self.page,
                            InspectorPage::RegistryDomains,
                            "Registry domains",
                        );
                    });

                    match self.page {
                        InspectorPage::Overview => {
                            egui::ScrollArea::vertical()
                                .id_salt("inspection_overview_scroll")
                                .show(ui, |ui| {
                                    show_overview(ui, &result.overview);

                                    self.global_error_pager.show(
                                        ui,
                                        "Global inspection error details",
                                        &result.snapshot.errors,
                                    );
                                });
                        }

                        InspectorPage::FolderTypes => {
                            egui::ScrollArea::vertical()
                                .id_salt("folder_types_scroll")
                                .show(ui, |ui| {
                                    self.folder_browser.show(
                                        ui,
                                        &result.snapshot,
                                        &result.folder_index,
                                    );
                                });
                        }

                        InspectorPage::Advanced => {
                            egui::ScrollArea::vertical()
                                .id_salt("advanced_settings_scroll")
                                .show(ui, |ui| {
                                    self.advanced_browser.show(ui, &result.snapshot.advanced);
                                });
                        }

                        InspectorPage::RegistryDomains => {
                            egui::ScrollArea::vertical()
                                .id_salt("registry_domains_scroll")
                                .show(ui, |ui| {
                                    self.registry_browser.show(ui, &result.snapshot);
                                });
                        }
                    }
                }

                InspectionState::Error(error) => {
                    ui.heading("Inspection failed");

                    ui.label(error);
                }
            }
        });
    }
}
