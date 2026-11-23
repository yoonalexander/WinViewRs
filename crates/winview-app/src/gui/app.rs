// winview-app/src/gui/app.rs

#[cfg(feature = "gui")]
use eframe::egui;

#[cfg(feature = "gui")]
pub struct WinViewApp {
    // App state
}

#[cfg(feature = "gui")]
impl Default for WinViewApp {
    fn default() -> Self {
        Self {}
    }
}

#[cfg(feature = "gui")]
impl eframe::App for WinViewApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("WinViewRs");
            ui.label("Minimal GUI - not yet implemented");
        });
    }
}
