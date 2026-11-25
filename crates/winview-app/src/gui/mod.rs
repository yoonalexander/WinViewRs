// winview-app/src/gui/mod.rs

#[cfg(feature = "gui")]
mod advanced;
#[cfg(feature = "gui")]
pub mod app;
#[cfg(feature = "gui")]
pub mod folder_types;
#[cfg(feature = "gui")]
pub mod inspection;
#[cfg(feature = "gui")]
pub mod inspector_tab;
#[cfg(feature = "gui")]
pub mod overview;

#[cfg(feature = "gui")]
pub fn run_gui() -> anyhow::Result<()> {
    use eframe::egui;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "WinViewRs",
        options,
        Box::new(|_creation_context| Ok(Box::new(app::WinViewApp::default()))),
    )
    .map_err(|error| anyhow::anyhow!("Failed to launch GUI: {error}"))
}
