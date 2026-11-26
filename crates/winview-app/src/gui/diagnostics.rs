// winview-app/src/gui/diagnostics.rs

use eframe::egui;

use winview_core::model::InspectionError;
use winview_core::RootKey;

const ERRORS_PER_PAGE: usize = 25;
const MAX_DIAGNOSTIC_CHARS: usize = 512;

#[derive(Default)]
pub(super) struct ErrorPager {
    page: usize,
}

impl ErrorPager {
    fn page_count(count: usize) -> usize {
        count.div_ceil(ERRORS_PER_PAGE).max(1)
    }

    pub(super) fn select_page(&mut self, page: usize, count: usize) {
        self.page = page.min(Self::page_count(count) - 1);
    }

    pub(super) fn page_range(&mut self, count: usize) -> std::ops::Range<usize> {
        self.select_page(self.page, count);

        let start = self.page * ERRORS_PER_PAGE;

        let end = start.saturating_add(ERRORS_PER_PAGE).min(count);

        start..end
    }

    pub(super) fn reset(&mut self) {
        self.page = 0;
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui, heading: &str, errors: &[InspectionError]) {
        ui.push_id(heading, |ui| {
            self.select_page(self.page, errors.len());

            ui.heading(heading);

            ui.label(format!("Recorded errors in this scope: {}", errors.len()));

            if errors.is_empty() {
                ui.label("No inspection errors were recorded in this scope.");

                ui.small(
                    "Zero recorded errors does not prove that the registry key \
                     exists or that all expected values were captured.",
                );

                return;
            }

            let pages = Self::page_count(errors.len());

            ui.horizontal(|ui| {
                let on_first = self.page == 0;

                if ui
                    .add_enabled(!on_first, egui::Button::new("Previous"))
                    .clicked()
                {
                    self.select_page(self.page.saturating_sub(1), errors.len());
                    ui.ctx().request_repaint();
                }

                if ui
                    .add_enabled(self.page + 1 < pages, egui::Button::new("Next"))
                    .clicked()
                {
                    self.select_page(self.page + 1, errors.len());
                    ui.ctx().request_repaint();
                }

                ui.label(format!("Page {} of {}", self.page + 1, pages));
            });

            let range = self.page_range(errors.len());

            for (offset, error) in errors[range.clone()].iter().enumerate() {
                let record_number = range.start + offset + 1;

                let row = diagnostic_row(error);

                ui.add_space(4.0);

                ui.strong(format!("Record {record_number}"));

                ui.label(format!("Hive: {}", row.hive));

                ui.label(format!("Key path: {}", row.key_path));

                ui.label(format!("Operation: {}", row.operation));

                ui.label(format!("Error: {}", row.message));
            }

            ui.add_space(4.0);

            ui.small(
                "Long diagnostic fields are truncated to 512 Unicode characters. \
                 The original captured records remain available through CLI exports.",
            );
        });
    }
}

fn diagnostic_preview(text: &str) -> String {
    if text.is_empty() {
        return "(Empty recorded text)".into();
    }

    let mut chars = text.chars();

    let mut preview: String = chars.by_ref().take(MAX_DIAGNOSTIC_CHARS).collect();

    if chars.next().is_some() {
        preview.push('…');
    }

    preview
}

#[derive(Debug, PartialEq, Eq)]
struct DiagnosticRow {
    hive: &'static str,
    key_path: String,
    operation: String,
    message: String,
}

fn diagnostic_row(error: &InspectionError) -> DiagnosticRow {
    DiagnosticRow {
        hive: match error.root {
            RootKey::Hkcu => "HKCU",
            RootKey::Hklm => "HKLM",
        },

        key_path: diagnostic_preview(&error.key_path),
        operation: diagnostic_preview(&error.operation),
        message: diagnostic_preview(&error.error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use winview_core::inspection::count_snapshot_read_errors;
    use winview_core::model::{InspectionSnapshot, ReadError, ValueReadResult};

    fn error(root: RootKey, path: &str, operation: &str, message: &str) -> InspectionError {
        InspectionError {
            key_path: path.into(),
            root,
            operation: operation.into(),
            error: message.into(),
        }
    }

    #[test]
    fn diagnostic_pagination_is_bounded() {
        let mut pager = ErrorPager::default();

        assert_eq!(pager.page_range(0), 0..0);
        assert_eq!(pager.page_range(1), 0..1);

        assert_eq!(ErrorPager::page_count(25), 1);
        assert_eq!(pager.page_range(25), 0..25);

        assert_eq!(ErrorPager::page_count(26), 2);
        assert_eq!(pager.page_range(26), 0..25);
        pager.select_page(1, 26);
        assert_eq!(pager.page_range(26), 25..26);

        assert_eq!(ErrorPager::page_count(51), 3);
        pager.select_page(0, 51);
        assert_eq!(pager.page_range(51), 0..25);
        pager.select_page(1, 51);
        assert_eq!(pager.page_range(51), 25..50);
        pager.select_page(2, 51);
        assert_eq!(pager.page_range(51), 50..51);

        // Previous-page navigation restores the correct range.
        pager.select_page(1, 51);
        assert_eq!(pager.page_range(51), 25..50);

        // Out-of-range page selection clamps safely.
        pager.select_page(99, 26);
        assert_eq!(pager.page, 1);
        assert_eq!(pager.page_range(26), 25..26);

        // A shrinking slice clamps the existing page correctly.
        pager.select_page(2, 51);
        assert_eq!(pager.page, 2);
        assert_eq!(pager.page_range(10), 0..10);
        assert_eq!(pager.page, 0);

        // reset() returns the pager to the first page.
        pager.select_page(2, 51);
        assert_eq!(pager.page, 2);
        pager.reset();
        assert_eq!(pager.page, 0);
        assert_eq!(pager.page_range(51), 0..25);

        // show() clamps the page immediately against a shrinking slice.
        let ctx = egui::Context::default();
        let many: Vec<InspectionError> = (0..51)
            .map(|i| {
                error(
                    RootKey::Hkcu,
                    "Software\\Test",
                    "open_key",
                    &format!("error {i}"),
                )
            })
            .collect();
        pager.select_page(2, many.len());
        assert_eq!(pager.page, 2);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            pager.show(ui, "Global inspection error details", &many[..10]);
        });
        output.textures_delta.clear();
        assert_eq!(pager.page, 0);

        // show() clamps safely against an empty slice.
        pager.select_page(2, many.len());
        let empty: [InspectionError; 0] = [];
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            pager.show(ui, "Global inspection error details", &empty);
        });
        output.textures_delta.clear();
        assert_eq!(pager.page, 0);
    }

    #[test]
    fn diagnostic_rows_preserve_recorded_fields() {
        let access = error(
            RootKey::Hkcu,
            r"Software\Test",
            "open_key",
            "Access is denied.",
        );

        let row = diagnostic_row(&access);
        assert_eq!(row.hive, "HKCU");
        assert_eq!(row.key_path, r"Software\Test");
        assert_eq!(row.operation, "open_key");
        assert_eq!(row.message, "Access is denied.");

        let value_limit = error(
            RootKey::Hklm,
            r"Software\Policies\Test",
            "value_size_limit",
            "Value exceeded the configured size limit.",
        );

        let row = diagnostic_row(&value_limit);
        assert_eq!(row.hive, "HKLM");
        assert_eq!(row.key_path, r"Software\Policies\Test");
        assert_eq!(row.operation, "value_size_limit");
        assert_eq!(row.message, "Value exceeded the configured size limit.");

        let snapshot_limit = error(
            RootKey::Hkcu,
            r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\Bags",
            "snapshot_size_limit",
            "Snapshot exceeded the configured size limit.",
        );

        let row = diagnostic_row(&snapshot_limit);
        assert_eq!(row.hive, "HKCU");
        assert_eq!(row.operation, "snapshot_size_limit");
        assert_eq!(row.message, "Snapshot exceeded the configured size limit.");
    }

    #[test]
    fn diagnostic_previews_are_unicode_safe() {
        assert_eq!(diagnostic_preview(""), "(Empty recorded text)");
        assert_eq!(diagnostic_preview("short"), "short");

        let exactly_512 = "a".repeat(512);
        assert_eq!(diagnostic_preview(&exactly_512), exactly_512);
        assert!(!diagnostic_preview(&exactly_512).contains('…'));

        let korean_513 = "한".repeat(513);
        let korean_preview = diagnostic_preview(&korean_513);
        assert_eq!(korean_preview, format!("{}…", "한".repeat(512)));
        assert_eq!(korean_preview.chars().count(), 513);

        let emoji_513 = "😀".repeat(513);
        let emoji_preview = diagnostic_preview(&emoji_513);
        assert_eq!(emoji_preview, format!("{}…", "😀".repeat(512)));
        assert_eq!(emoji_preview.chars().count(), 513);
    }

    #[test]
    fn diagnostic_rendering_preserves_recorded_errors() {
        let mut snapshot = InspectionSnapshot::default();

        snapshot.errors.push(error(
            RootKey::Hkcu,
            r"Software\Test",
            "open_key",
            "Access is denied.",
        ));

        // Identical duplicate record must remain a distinct entry.
        snapshot.errors.push(error(
            RootKey::Hkcu,
            r"Software\Test",
            "open_key",
            "Access is denied.",
        ));

        snapshot.errors.push(error(
            RootKey::Hklm,
            r"Software\Policies\Test",
            "value_size_limit",
            "Value exceeded the configured size limit.",
        ));

        // A failed value observation that is not in the recorded error list.
        snapshot.advanced.values.insert(
            "Missing".into(),
            ValueReadResult::Failure(ReadError::ValueNotFound {
                name: "Missing".into(),
            }),
        );

        let before = count_snapshot_read_errors(&snapshot);
        let before_json = serde_json::to_string(&snapshot).unwrap();

        let mut pager = ErrorPager::default();

        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            pager.show(ui, "Global inspection error details", &snapshot.errors);
        });
        output.textures_delta.clear();

        // Duplicate recorded errors remain distinct entries in order.
        let rows: Vec<DiagnosticRow> = snapshot.errors.iter().map(diagnostic_row).collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], rows[1]);
        assert_eq!(rows[0].hive, "HKCU");
        assert_eq!(rows[2].hive, "HKLM");
        assert_eq!(rows[2].operation, "value_size_limit");

        // Failed value observations do not become extra diagnostics.
        assert_eq!(snapshot.errors.len(), 3);

        let after = count_snapshot_read_errors(&snapshot);
        let after_json = serde_json::to_string(&snapshot).unwrap();

        assert_eq!(before, after);
        assert_eq!(before_json, after_json);
    }
}
