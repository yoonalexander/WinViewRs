use std::time::Duration;

use eframe::egui;

use winview_core::model::InspectionStatus;

use winview_core::RegistryReader;

use super::inspection::InspectionState;

#[derive(Default)]
pub struct WinViewApp {
    inspection: InspectionState,
}

impl eframe::App for WinViewApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.inspection.poll();

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("WinViewRs");

            ui.label("Windows Explorer Configuration Inspector");

            ui.separator();

            ui.label("Read-only mode");

            ui.add_space(8.0);

            let loading = self.inspection.is_loading();

            let button_label = if loading {
                "Inspecting..."
            } else if matches!(&self.inspection, InspectionState::Idle) {
                "Run inspection"
            } else {
                "Refresh inspection"
            };

            if ui
                .add_enabled(!loading, egui::Button::new(button_label))
                .clicked()
            {
                self.inspection.start(|| {
                    let reader = winview_registry::WindowsRegistryReader;

                    reader.inspect_all().map_err(|error| error.to_string())
                });
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
                }

                InspectionState::Error(error) => {
                    ui.heading("Inspection failed");

                    ui.label(error);
                }
            }
        });
    }
}
