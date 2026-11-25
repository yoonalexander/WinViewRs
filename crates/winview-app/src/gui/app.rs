#[cfg(feature = "gui")]
use eframe::egui;

#[cfg(feature = "gui")]
#[derive(Default)]
pub struct WinViewApp {
    // Future application state.
}

#[cfg(feature = "gui")]
impl eframe::App for WinViewApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("WinViewRs");

            ui.label("Windows Explorer Configuration Inspector");

            ui.separator();

            ui.label("Read-only mode");

            ui.label("Registry inspection is not yet connected.");
        });
    }
}
