// winview-app/src/gui/style.rs

use eframe::egui;

/// Shared visual style configuration for the application.
/// Configured once at startup and applied globally.
pub fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.style_of(egui::Theme::Light)).clone();

    // Typography
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::new(24.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::new(14.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::new(13.0, egui::FontFamily::Monospace),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        egui::FontId::new(12.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::new(14.0, egui::FontFamily::Proportional),
    );

    // Spacing
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.window_margin = egui::Margin::symmetric(16, 16);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);
    style.spacing.indent = 16.0;

    // Corner radius
    let radius = 6.0;
    style.visuals.window_corner_radius = radius.into();
    style.visuals.menu_corner_radius = radius.into();

    // Restrained blue accent for selection and primary actions
    let accent = egui::Color32::from_rgb(0, 102, 204);
    style.visuals.selection.bg_fill = accent.gamma_multiply(0.15);
    style.visuals.selection.stroke = egui::Stroke::new(1.0, accent);
    style.visuals.widgets.active.bg_fill = accent.gamma_multiply(0.12);
    style.visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, accent);
    style.visuals.widgets.hovered.bg_fill = accent.gamma_multiply(0.08);
    style.visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, accent);
    style.visuals.widgets.inactive.bg_fill = accent.gamma_multiply(0.05);

    // Links use accent color
    style.visuals.hyperlink_color = accent;

    // Consistent button sizing
    style.spacing.interact_size = egui::vec2(100.0, 32.0);

    ctx.set_style_of(egui::Theme::Light, style.clone());
    ctx.set_style_of(egui::Theme::Dark, style);
}

/// Semantic color helpers derived from the active theme.
pub struct ThemeColors {
    pub background: egui::Color32,
    pub panel_background: egui::Color32,
    pub text: egui::Color32,
    pub text_secondary: egui::Color32,
    pub border: egui::Color32,
    pub accent: egui::Color32,
    pub success: egui::Color32,
    pub warning: egui::Color32,
    pub error: egui::Color32,
}

impl ThemeColors {
    pub fn from_style(style: &egui::Style) -> Self {
        let visuals = &style.visuals;
        let is_dark = visuals.dark_mode;

        if is_dark {
            Self {
                background: visuals.panel_fill,
                panel_background: visuals.extreme_bg_color,
                text: visuals.text_color(),
                text_secondary: visuals.weak_text_color(),
                border: visuals.window_stroke.color,
                accent: egui::Color32::from_rgb(80, 160, 255),
                success: egui::Color32::from_rgb(80, 200, 120),
                warning: egui::Color32::from_rgb(255, 180, 60),
                error: egui::Color32::from_rgb(255, 100, 100),
            }
        } else {
            Self {
                background: visuals.panel_fill,
                panel_background: visuals.extreme_bg_color,
                text: visuals.text_color(),
                text_secondary: visuals.weak_text_color(),
                border: visuals.window_stroke.color,
                accent: egui::Color32::from_rgb(0, 102, 204),
                success: egui::Color32::from_rgb(0, 160, 80),
                warning: egui::Color32::from_rgb(200, 140, 0),
                error: egui::Color32::from_rgb(200, 60, 60),
            }
        }
    }
}

/// Reusable presentation helpers.
pub mod helpers {
    use super::*;

    /// Render a summary card with a title, value, and optional subtitle.
    pub fn summary_card(ui: &mut egui::Ui, title: &str, value: &str, subtitle: Option<&str>) {
        let colors = ThemeColors::from_style(&ui.style());

        egui::Frame::new()
            .fill(colors.panel_background)
            .stroke(egui::Stroke::new(1.0, colors.border))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(title)
                            .size(12.0)
                            .color(colors.text_secondary),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(value)
                            .size(22.0)
                            .strong()
                            .color(colors.text),
                    );
                    if let Some(sub) = subtitle {
                        ui.add_space(2.0);
                        ui.label(
                            egui::RichText::new(sub)
                                .size(11.0)
                                .color(colors.text_secondary),
                        );
                    }
                });
            });
    }

    /// Render a status badge with color coding.
    pub fn status_badge(ui: &mut egui::Ui, text: &str, kind: StatusKind) {
        let colors = ThemeColors::from_style(&ui.style());
        let (bg, fg) = match kind {
            StatusKind::Success => (colors.success.gamma_multiply(0.15), colors.success),
            StatusKind::Warning => (colors.warning.gamma_multiply(0.15), colors.warning),
            StatusKind::Error => (colors.error.gamma_multiply(0.15), colors.error),
            StatusKind::Info => (colors.accent.gamma_multiply(0.15), colors.accent),
            StatusKind::Neutral => (colors.border.gamma_multiply(0.15), colors.text_secondary),
        };

        egui::Frame::new()
            .fill(bg)
            .corner_radius(egui::CornerRadius::same(4))
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.label(egui::RichText::new(text).size(12.0).strong().color(fg));
            });
    }

    /// Render a section heading with consistent styling.
    pub fn section_heading(ui: &mut egui::Ui, text: &str) {
        let colors = ThemeColors::from_style(&ui.style());
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(text)
                .size(17.0)
                .strong()
                .color(colors.text),
        );
        ui.add_space(8.0);
    }

    /// Render a page title with description.
    pub fn page_header(ui: &mut egui::Ui, title: &str, description: &str) {
        let colors = ThemeColors::from_style(&ui.style());
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(title)
                    .size(24.0)
                    .strong()
                    .color(colors.text),
            );
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(description)
                    .size(13.0)
                    .color(colors.text_secondary),
            );
        });
        ui.add_space(12.0);
    }

    /// Render a key-value pair with proper text wrapping and optional monospace.
    pub fn key_value_row(ui: &mut egui::Ui, key: &str, value: &str, monospace: bool) {
        let colors = ThemeColors::from_style(&ui.style());
        ui.horizontal(|ui| {
            ui.set_min_width(160.0);
            ui.label(
                egui::RichText::new(key)
                    .size(13.0)
                    .color(colors.text_secondary),
            );
            ui.add_space(12.0);
            if monospace {
                ui.monospace(egui::RichText::new(value).size(13.0).color(colors.text));
            } else {
                ui.label(egui::RichText::new(value).size(13.0).color(colors.text));
            }
        });
    }

    /// Render a read-only text field that supports selection and wrapping.
    pub fn readonly_text(ui: &mut egui::Ui, text: &str, monospace: bool) {
        let colors = ThemeColors::from_style(&ui.style());
        let text_style = if monospace {
            egui::TextStyle::Monospace
        } else {
            egui::TextStyle::Body
        };
        ui.add(
            egui::TextEdit::multiline(&mut text.to_string())
                .font(text_style)
                .desired_width(f32::INFINITY)
                .desired_rows(1)
                .interactive(false)
                .frame(egui::Frame::new().fill(egui::Color32::TRANSPARENT)),
        );
    }

    /// Render a clickable navigation item with consistent styling.
    pub fn nav_item(ui: &mut egui::Ui, selected: bool, label: &str) -> egui::Response {
        let colors = ThemeColors::from_style(&ui.style());
        let response =
            ui.allocate_response(egui::vec2(ui.available_width(), 40.0), egui::Sense::click());

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

        let text_pos = response.rect.left_top() + egui::vec2(12.0, 10.0);
        ui.painter().text(
            text_pos,
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::new(14.0, egui::FontFamily::Proportional),
            if selected { colors.accent } else { colors.text },
        );

        response
    }

    pub fn nav_item_small(ui: &mut egui::Ui, selected: bool, label: &str) -> egui::Response {
        let colors = ThemeColors::from_style(&ui.style());
        let response = ui.allocate_response(
            egui::vec2(ui.available_width().min(140.0), 32.0),
            egui::Sense::click(),
        );

        let bg = if selected {
            colors.accent.gamma_multiply(0.12)
        } else if response.hovered() {
            colors.accent.gamma_multiply(0.06)
        } else {
            egui::Color32::TRANSPARENT
        };

        if bg != egui::Color32::TRANSPARENT {
            ui.painter().rect_filled(response.rect, 4.0, bg);
        }

        if selected {
            ui.painter().rect_stroke(
                response.rect,
                4.0,
                egui::Stroke::new(2.0, colors.accent),
                egui::StrokeKind::Outside,
            );
        }

        let text_pos = response.rect.center();
        ui.painter().text(
            text_pos,
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::new(13.0, egui::FontFamily::Proportional),
            if selected { colors.accent } else { colors.text },
        );

        response
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StatusKind {
    Success,
    Warning,
    Error,
    Info,
    Neutral,
}

/// Width breakpoints for responsive layout.
pub const SIDEBAR_WIDTH: f32 = 200.0;
pub const SIDEBAR_MIN_WIDTH: f32 = 180.0;
pub const SIDEBAR_MAX_WIDTH: f32 = 210.0;
pub const FOLDER_LIST_WIDTH: f32 = 320.0;
pub const FOLDER_LIST_MIN_WIDTH: f32 = 280.0;
pub const FOLDER_LIST_MAX_WIDTH: f32 = 340.0;
pub const WIDE_WINDOW_THRESHOLD: f32 = 900.0;

/// Get the sidebar width based on available space.
pub fn sidebar_width(available: f32) -> f32 {
    if available >= WIDE_WINDOW_THRESHOLD {
        SIDEBAR_WIDTH.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH)
    } else {
        0.0
    }
}

/// Get the folder list width based on available space.
pub fn folder_list_width(available: f32) -> f32 {
    if available >= WIDE_WINDOW_THRESHOLD {
        FOLDER_LIST_WIDTH.clamp(FOLDER_LIST_MIN_WIDTH, FOLDER_LIST_MAX_WIDTH)
    } else {
        available * 0.5
    }
}
