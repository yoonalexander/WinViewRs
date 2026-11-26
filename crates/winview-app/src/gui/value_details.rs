// winview-app/src/gui/value_details.rs

use std::fmt::Write;

use eframe::egui;

use winview_core::model::{
    DecodingStatus, ObservationSource, ReadError, RegistryValue, ValueReadResult,
};
use winview_core::RootKey;

use super::style::helpers;
use super::style::{ThemeColors, WIDE_WINDOW_THRESHOLD};

const MAX_HEX_BYTES: usize = 64;
const MAX_TEXT_CHARS: usize = 160;
const MAX_MULTI_STRING_ITEMS: usize = 8;

const UNAVAILABLE: &str = "(Unavailable)";

fn hex_preview(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "(Empty payload)".into();
    }

    let mut output = String::new();

    for (index, byte) in bytes.iter().take(MAX_HEX_BYTES).enumerate() {
        if index > 0 {
            output.push(' ');
        }

        write!(&mut output, "{byte:02X}").expect("Writing to String cannot fail");
    }

    if bytes.len() > MAX_HEX_BYTES {
        write!(
            &mut output,
            " … ({} additional bytes)",
            bytes.len() - MAX_HEX_BYTES
        )
        .expect("Writing to String cannot fail");
    }

    output
}

fn text_preview(value: &str) -> String {
    if value.is_empty() {
        return "(Empty string)".into();
    }

    let mut chars = value.chars();

    let mut output: String = chars.by_ref().take(MAX_TEXT_CHARS).collect();

    if chars.next().is_some() {
        output.push('…');
    }

    output
}

fn decoded_preview(value: &RegistryValue) -> String {
    match value {
        RegistryValue::String(text) => text_preview(text),
        RegistryValue::ExpandString(text) => text_preview(text),
        RegistryValue::Dword(number) => number.to_string(),
        RegistryValue::Qword(number) => number.to_string(),
        RegistryValue::Binary(bytes) => hex_preview(bytes),
        RegistryValue::MultiString(items) => {
            if items.is_empty() {
                return "(Empty multi-string)".into();
            }

            let mut output = String::new();

            for (index, item) in items.iter().take(MAX_MULTI_STRING_ITEMS).enumerate() {
                if index > 0 {
                    output.push_str(" | ");
                }

                output.push_str(&text_preview(item));
            }

            if items.len() > MAX_MULTI_STRING_ITEMS {
                write!(
                    &mut output,
                    " … ({} additional entries)",
                    items.len() - MAX_MULTI_STRING_ITEMS
                )
                .expect("Writing to String cannot fail");
            }

            output
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct DetailSection {
    pub title: &'static str,
    pub rows: Vec<DetailRow>,
}

#[derive(Debug, Clone)]
pub(super) struct DetailRow {
    pub label: &'static str,
    pub value: String,
    pub monospace: bool,
}

impl DetailRow {
    fn new(label: &'static str, value: impl Into<String>) -> Self {
        Self {
            label,
            value: value.into(),
            monospace: false,
        }
    }

    fn monospace(label: &'static str, value: impl Into<String>) -> Self {
        Self {
            label,
            value: value.into(),
            monospace: true,
        }
    }
}

fn hive_label(root: RootKey) -> &'static str {
    match root {
        RootKey::Hkcu => "HKCU",
        RootKey::Hklm => "HKLM",
    }
}

fn source_label(source: ObservationSource) -> &'static str {
    match source {
        ObservationSource::HkcuFolderTypes => "HKCU FolderTypes",
        ObservationSource::HklmFolderTypes => "HKLM FolderTypes",
        ObservationSource::HkcuBags => "HKCU Bags",
        ObservationSource::HkcuStreams => "HKCU Streams",
        ObservationSource::HkcuAdvanced => "HKCU Advanced",
        ObservationSource::HkcuClsid => "HKCU CLSID",
        ObservationSource::HkcuPolicies => "HKCU Policies",
        ObservationSource::HklmPolicies => "HKLM Policies",
    }
}

fn read_error_label(error: &ReadError) -> (&'static str, &str) {
    match error {
        ReadError::KeyNotFound { path } => ("Key not found", path.as_str()),
        ReadError::ValueNotFound { name } => ("Value not found", name.as_str()),
        ReadError::AccessDenied { path } => ("Access denied", path.as_str()),
        ReadError::Other(message) => ("Read failed", message.as_str()),
    }
}

fn build_detail_sections(name: &str, observation: &ValueReadResult) -> Vec<DetailSection> {
    let display_name = if name.is_empty() {
        "(Default)".to_string()
    } else {
        text_preview(name)
    };

    let mut sections = Vec::new();

    // Registry identity
    let mut identity_rows = vec![DetailRow::new("Value name", display_name)];

    match observation {
        ValueReadResult::Success(raw) => {
            identity_rows.push(DetailRow::new("Registry hive", hive_label(raw.root)));
            identity_rows.push(DetailRow::new(
                "Captured key path",
                text_preview(&raw.key_path),
            ));
            identity_rows.push(DetailRow::new(
                "Observation source",
                source_label(raw.source),
            ));
        }
        ValueReadResult::Failure(_) => {
            identity_rows.push(DetailRow::new("Registry hive", UNAVAILABLE));
            identity_rows.push(DetailRow::new("Captured key path", UNAVAILABLE));
            identity_rows.push(DetailRow::new("Observation source", UNAVAILABLE));
        }
    }

    sections.push(DetailSection {
        title: "Registry identity",
        rows: identity_rows,
    });

    // Read and decoding status
    let mut status_rows = Vec::new();

    match observation {
        ValueReadResult::Success(raw) => {
            status_rows.push(DetailRow::new(
                "Registry type ID",
                raw.raw_type_id.to_string(),
            ));
            status_rows.push(DetailRow::new(
                "Raw payload size",
                format!("{} bytes", raw.raw_bytes.len()),
            ));

            match &raw.decoding_status {
                DecodingStatus::Decoded => {
                    status_rows.push(DetailRow::new("Decoding status", "Decoded"));
                }
                DecodingStatus::Malformed { reason } => {
                    status_rows.push(DetailRow::new("Decoding status", "Malformed data"));
                    status_rows.push(DetailRow::new("Malformed reason", text_preview(reason)));
                }
                DecodingStatus::UnknownType => {
                    status_rows.push(DetailRow::new("Decoding status", "Unknown registry type"));
                }
            }
        }
        ValueReadResult::Failure(error) => {
            let (status, detail) = read_error_label(error);
            status_rows.push(DetailRow::new("Read status", status));
            status_rows.push(DetailRow::new("Error details", text_preview(detail)));
            status_rows.push(DetailRow::new("Registry type ID", UNAVAILABLE));
            status_rows.push(DetailRow::new("Raw payload size", UNAVAILABLE));
        }
    }

    sections.push(DetailSection {
        title: "Read and decoding status",
        rows: status_rows,
    });

    // Raw payload preview
    let mut payload_rows = Vec::new();

    match observation {
        ValueReadResult::Success(raw) => {
            payload_rows.push(DetailRow::monospace(
                "Raw payload preview (hex)",
                hex_preview(&raw.raw_bytes),
            ));
        }
        ValueReadResult::Failure(_) => {
            payload_rows.push(DetailRow::new("Raw payload preview (hex)", UNAVAILABLE));
        }
    }

    sections.push(DetailSection {
        title: "Raw payload preview",
        rows: payload_rows,
    });

    // Decoded representation
    let mut decoded_rows = Vec::new();

    let decoded = match observation {
        ValueReadResult::Success(raw) => match (&raw.decoding_status, &raw.decoded) {
            (DecodingStatus::Decoded, Some(value)) => decoded_preview(value),
            _ => UNAVAILABLE.to_string(),
        },
        ValueReadResult::Failure(_) => UNAVAILABLE.to_string(),
    };

    decoded_rows.push(DetailRow::new("Decoded representation", decoded));

    sections.push(DetailSection {
        title: "Decoded representation",
        rows: decoded_rows,
    });

    sections
}

pub(super) fn show_value_details(ui: &mut egui::Ui, name: &str, observation: &ValueReadResult) {
    let colors = ThemeColors::from_style(&ui.style());
    let available_width = ui.available_width();
    let is_wide = available_width >= WIDE_WINDOW_THRESHOLD;

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);

    helpers::section_heading(ui, "Value details");

    let sections = build_detail_sections(name, observation);

    for section in sections {
        if is_wide {
            // Two-column layout
            egui::Grid::new(section.title)
                .num_columns(2)
                .striped(true)
                .min_col_width(180.0)
                .show(ui, |ui| {
                    for row in section.rows {
                        ui.label(
                            egui::RichText::new(row.label)
                                .size(13.0)
                                .color(colors.text_secondary),
                        );

                        if row.monospace {
                            ui.monospace(
                                egui::RichText::new(&row.value)
                                    .size(13.0)
                                    .color(colors.text),
                            );
                        } else {
                            ui.label(
                                egui::RichText::new(&row.value)
                                    .size(13.0)
                                    .color(colors.text),
                            );
                        }
                        ui.end_row();
                    }
                });
        } else {
            // Stacked layout
            for row in section.rows {
                ui.horizontal(|ui| {
                    ui.set_min_width(160.0);
                    ui.label(
                        egui::RichText::new(row.label)
                            .size(13.0)
                            .color(colors.text_secondary),
                    );
                    ui.add_space(12.0);

                    if row.monospace {
                        ui.monospace(
                            egui::RichText::new(&row.value)
                                .size(13.0)
                                .color(colors.text),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new(&row.value)
                                .size(13.0)
                                .color(colors.text),
                        );
                    }
                });
            }
        }

        ui.add_space(12.0);
    }

    ui.small(
        egui::RichText::new("Previews are bounded and are not a complete raw payload dump.")
            .color(colors.text_secondary),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    use winview_core::model::{InspectionSnapshot, KeySnapshot, RawRegistryValue};

    fn success(
        type_id: u32,
        bytes: Vec<u8>,
        decoded: Option<RegistryValue>,
        status: DecodingStatus,
    ) -> ValueReadResult {
        ValueReadResult::Success(RawRegistryValue {
            key_path: r"Software\Test".into(),
            value_name: "Mode".into(),
            root: RootKey::Hkcu,
            raw_type_id: type_id,
            raw_bytes: bytes,
            decoded,
            decoding_status: status,
            source: ObservationSource::HkcuBags,
        })
    }

    fn row_value<'a>(rows: &'a [DetailRow], label: &str) -> &'a str {
        rows.iter()
            .find(|row| row.label == label)
            .map(|row| row.value.as_str())
            .unwrap_or_else(|| panic!("missing detail row: {label}"))
    }

    #[test]
    fn hex_preview_is_bounded() {
        assert_eq!(hex_preview(&[]), "(Empty payload)");

        assert_eq!(hex_preview(&[0x00, 0x1F, 0xAB]), "00 1F AB");

        let exactly_64: Vec<u8> = (0..64).map(|i| i as u8).collect();
        let preview_64 = hex_preview(&exactly_64);
        assert!(!preview_64.contains('…'));
        assert!(!preview_64.contains("additional bytes"));
        assert_eq!(preview_64.split(' ').count(), 64);

        let sixty_five: Vec<u8> = (0..65).map(|i| i as u8).collect();
        let preview_65 = hex_preview(&sixty_five);
        assert!(preview_65.contains("… (1 additional bytes)"));
        let hex_tokens: Vec<&str> = preview_65
            .split(' ')
            .take_while(|token| token.chars().all(|c| c.is_ascii_hexdigit()))
            .collect();
        assert_eq!(hex_tokens.len(), 64);

        let large: Vec<u8> = (0..1000).map(|i| (i % 256) as u8).collect();
        let preview_large = hex_preview(&large);
        assert!(preview_large.contains("… (936 additional bytes)"));
        assert!(preview_large.len() < 64 * 3 + 32);
    }

    #[test]
    fn text_preview_is_unicode_safe() {
        assert_eq!(text_preview(""), "(Empty string)");
        assert_eq!(text_preview("abc"), "abc");

        let exactly_160 = "a".repeat(160);
        assert_eq!(text_preview(&exactly_160), exactly_160);
        assert!(!text_preview(&exactly_160).contains('…'));

        let korean_161 = "한".repeat(161);
        let preview = text_preview(&korean_161);
        assert_eq!(preview, format!("{}…", "한".repeat(160)));
        assert_eq!(preview.chars().count(), 161);
    }

    #[test]
    fn decoded_preview_preserves_unsigned_numbers() {
        assert_eq!(decoded_preview(&RegistryValue::Dword(0)), "0");
        assert_eq!(
            decoded_preview(&RegistryValue::Dword(u32::MAX)),
            "4294967295"
        );
        assert_eq!(decoded_preview(&RegistryValue::Qword(0)), "0");
        assert_eq!(
            decoded_preview(&RegistryValue::Qword(u64::MAX)),
            "18446744073709551615"
        );
    }

    #[test]
    fn decoded_preview_bounds_complex_values() {
        assert_eq!(
            decoded_preview(&RegistryValue::String("hello".into())),
            "hello"
        );
        assert_eq!(
            decoded_preview(&RegistryValue::ExpandString("%PATH%".into())),
            "%PATH%"
        );
        assert_eq!(
            decoded_preview(&RegistryValue::Binary(vec![0x01, 0x02])),
            "01 02"
        );

        assert_eq!(
            decoded_preview(&RegistryValue::MultiString(vec![])),
            "(Empty multi-string)"
        );

        let many: Vec<String> = (0..12).map(|i| format!("item{i}")).collect();
        let preview = decoded_preview(&RegistryValue::MultiString(many));
        assert!(preview.contains("item0"));
        assert!(preview.contains("item7"));
        assert!(!preview.contains("item8"));
        assert!(preview.contains("… (4 additional entries)"));

        let long_items = vec!["x".repeat(200)];
        let long_preview = decoded_preview(&RegistryValue::MultiString(long_items));
        assert!(long_preview.ends_with('…'));
        assert_eq!(long_preview.chars().count(), 161);
    }

    #[test]
    fn value_details_preserve_observation_states() {
        let decoded = success(
            4,
            1u32.to_le_bytes().to_vec(),
            Some(RegistryValue::Dword(1)),
            DecodingStatus::Decoded,
        );
        let sections = build_detail_sections("Mode", &decoded);
        let identity_rows = &sections[0].rows;
        let status_rows = &sections[1].rows;
        let payload_rows = &sections[2].rows;
        let decoded_rows = &sections[3].rows;

        assert_eq!(row_value(identity_rows, "Value name"), "Mode");
        assert_eq!(row_value(status_rows, "Decoding status"), "Decoded");
        assert_eq!(row_value(decoded_rows, "Decoded representation"), "1");
        assert_eq!(row_value(status_rows, "Registry type ID"), "4");
        assert!(row_value(payload_rows, "Raw payload preview (hex)").contains("01 00 00 00"));

        let unknown = success(99, vec![1, 2, 3], None, DecodingStatus::UnknownType);
        let sections = build_detail_sections("Unknown", &unknown);
        let status_rows = &sections[1].rows;
        let decoded_rows = &sections[3].rows;
        assert_eq!(
            row_value(status_rows, "Decoding status"),
            "Unknown registry type"
        );
        assert_eq!(row_value(status_rows, "Registry type ID"), "99");
        assert_eq!(
            row_value(decoded_rows, "Decoded representation"),
            UNAVAILABLE
        );

        let malformed = success(
            4,
            vec![1],
            None,
            DecodingStatus::Malformed {
                reason: "Invalid DWORD length".into(),
            },
        );
        let sections = build_detail_sections("Malformed", &malformed);
        let status_rows = &sections[1].rows;
        let decoded_rows = &sections[3].rows;
        assert_eq!(row_value(status_rows, "Decoding status"), "Malformed data");
        assert_eq!(
            row_value(status_rows, "Malformed reason"),
            "Invalid DWORD length"
        );
        assert_eq!(row_value(status_rows, "Registry type ID"), "4");
        assert_eq!(
            row_value(decoded_rows, "Decoded representation"),
            UNAVAILABLE
        );

        let failed = ValueReadResult::Failure(ReadError::AccessDenied {
            path: r"Software\Test".into(),
        });
        let sections = build_detail_sections("Failed", &failed);
        let identity_rows = &sections[0].rows;
        let status_rows = &sections[1].rows;
        let decoded_rows = &sections[3].rows;
        assert_eq!(row_value(identity_rows, "Value name"), "Failed");
        assert_eq!(row_value(status_rows, "Read status"), "Access denied");
        assert_eq!(row_value(status_rows, "Error details"), r"Software\Test");
        assert_eq!(row_value(status_rows, "Registry type ID"), UNAVAILABLE);
        assert_eq!(row_value(status_rows, "Raw payload size"), UNAVAILABLE);
        assert_eq!(
            row_value(decoded_rows, "Decoded representation"),
            UNAVAILABLE
        );

        let mut key = KeySnapshot::default();
        key.values.insert("Mode".into(), decoded);
        key.values.insert("Unknown".into(), unknown);
        key.values.insert("Malformed".into(), malformed);
        key.values.insert("Failed".into(), failed);

        assert_eq!(key.values.len(), 4);

        let before = serde_json::to_string(&key).unwrap();

        for (name, observation) in &key.values {
            let _ = build_detail_sections(name, observation);
        }

        let ctx = egui::Context::default();
        for (name, observation) in &key.values {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                show_value_details(ui, name, observation);
            });
            output.textures_delta.clear();
        }

        let after = serde_json::to_string(&key).unwrap();

        assert_eq!(before, after);
        assert!(key.read_errors.is_empty());

        let snapshot = InspectionSnapshot::default();
        assert!(snapshot.bags.is_empty());
    }

    #[test]
    fn build_detail_sections_organizes_content_correctly() {
        let observation = success(
            4,
            1u32.to_le_bytes().to_vec(),
            Some(RegistryValue::Dword(1)),
            DecodingStatus::Decoded,
        );

        let sections = build_detail_sections("TestValue", &observation);

        assert_eq!(sections.len(), 4);
        assert_eq!(sections[0].title, "Registry identity");
        assert_eq!(sections[1].title, "Read and decoding status");
        assert_eq!(sections[2].title, "Raw payload preview");
        assert_eq!(sections[3].title, "Decoded representation");

        // Check identity section
        let labels: Vec<&str> = sections[0].rows.iter().map(|r| r.label).collect();
        assert!(labels.contains(&"Value name"));
        assert!(labels.contains(&"Registry hive"));
        assert!(labels.contains(&"Captured key path"));
        assert!(labels.contains(&"Observation source"));

        // Check status section
        let labels: Vec<&str> = sections[1].rows.iter().map(|r| r.label).collect();
        assert!(labels.contains(&"Registry type ID"));
        assert!(labels.contains(&"Raw payload size"));
        assert!(labels.contains(&"Decoding status"));
    }

    #[test]
    fn build_detail_sections_handles_failure_observation() {
        let observation = ValueReadResult::Failure(ReadError::ValueNotFound {
            name: "Missing".into(),
        });

        let sections = build_detail_sections("Missing", &observation);

        assert_eq!(sections.len(), 4);

        let identity_rows = &sections[0].rows;
        assert_eq!(row_value(identity_rows, "Value name"), "Missing");
        assert_eq!(row_value(identity_rows, "Registry hive"), UNAVAILABLE);

        let status_rows = &sections[1].rows;
        assert_eq!(row_value(status_rows, "Read status"), "Value not found");
        assert_eq!(row_value(status_rows, "Registry type ID"), UNAVAILABLE);
    }
}
