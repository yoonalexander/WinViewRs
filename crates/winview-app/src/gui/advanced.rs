use eframe::egui;

use winview_core::model::{DecodingStatus, KeySnapshot, ReadError, RegistryValue, ValueReadResult};

use winview_core::registry::keys::{advanced_values::*, paths::ADVANCED};

const SETTINGS: [(&str, &str); 7] = [
    (HIDE_FILE_EXT, "File name extensions"),
    (USE_COMPACT_MODE, "Compact mode"),
    (AUTO_CHECK_SELECT, "Item selection checkboxes"),
    (HIDDEN, "Hidden files and folders"),
    (LAUNCH_TO, "Explorer launch location"),
    (FULL_ROW_SELECT, "Full-row selection"),
    (NO_STR_CMP_LOGICAL, "Logical filename sorting"),
];

#[derive(Debug, PartialEq, Eq)]
struct AdvancedRow {
    name: &'static str,
    label: &'static str,
    value: String,
    status: &'static str,
}

fn describe_observation(observation: Option<&ValueReadResult>) -> (String, &'static str) {
    let unavailable = || "Unavailable".to_string();

    match observation {
        None => (unavailable(), "Not captured"),

        Some(ValueReadResult::Failure(error)) => {
            let status = match error {
                ReadError::ValueNotFound { .. } => "Value not found",
                ReadError::KeyNotFound { .. } => "Key not found",
                ReadError::AccessDenied { .. } => "Access denied",
                ReadError::Other(_) => "Read failed",
            };

            (unavailable(), status)
        }

        Some(ValueReadResult::Success(raw)) => match &raw.decoding_status {
            DecodingStatus::Malformed { .. } => (unavailable(), "Malformed data"),

            DecodingStatus::UnknownType => (unavailable(), "Unknown registry type"),

            DecodingStatus::Decoded => match &raw.decoded {
                Some(RegistryValue::Dword(value)) if raw.raw_type_id == 4 => {
                    (value.to_string(), "Captured")
                }

                _ => (unavailable(), "Unexpected value type"),
            },
        },
    }
}

fn advanced_rows(advanced: &KeySnapshot) -> [AdvancedRow; 7] {
    SETTINGS.map(|(name, label)| {
        let (value, status) = describe_observation(advanced.values.get(name));

        AdvancedRow {
            name,
            label,
            value,
            status,
        }
    })
}

pub(super) fn show_advanced(ui: &mut egui::Ui, advanced: &KeySnapshot) {
    ui.heading("Advanced Explorer settings");

    ui.label(format!("HKCU\\{ADVANCED}"));

    ui.label(format!(
        "Key-level recorded errors: {}",
        advanced.read_errors.len()
    ));

    ui.small(
        "Recorded registry values only. \
         These may not represent effective \
         Explorer settings.",
    );

    ui.add_space(8.0);

    egui::Grid::new("advanced_settings_grid")
        .num_columns(3)
        .striped(true)
        .show(ui, |ui| {
            ui.strong("Setting");
            ui.strong("Value");
            ui.strong("Read status");

            ui.end_row();

            for row in advanced_rows(advanced) {
                ui.vertical(|ui| {
                    ui.label(row.label);
                    ui.small(row.name);
                });

                ui.label(row.value);
                ui.label(row.status);

                ui.end_row();
            }
        });

    ui.add_space(8.0);

    ui.small(
        "Not captured may indicate an absent key, \
         an inspection limitation, or an omitted \
         observation. Missing values do not \
         imply disabled settings.",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    use winview_core::inspection::count_key_read_errors;
    use winview_core::model::{ObservationSource, RawRegistryValue};
    use winview_core::RootKey;

    fn dword_observation(name: &str, value: u32) -> ValueReadResult {
        ValueReadResult::Success(RawRegistryValue {
            key_path: ADVANCED.into(),
            value_name: name.into(),
            root: RootKey::Hkcu,
            raw_type_id: 4,
            raw_bytes: value.to_le_bytes().to_vec(),
            decoded: Some(RegistryValue::Dword(value)),
            decoding_status: DecodingStatus::Decoded,
            source: ObservationSource::HkcuAdvanced,
        })
    }

    fn empty_snapshot() -> KeySnapshot {
        KeySnapshot::default()
    }

    #[test]
    fn advanced_rows_preserve_supported_setting_order() {
        let snapshot = empty_snapshot();
        let rows = advanced_rows(&snapshot);
        assert_eq!(rows.len(), 7);
        let names: Vec<&str> = rows.iter().map(|r| r.name).collect();
        assert_eq!(
            names,
            vec![
                "HideFileExt",
                "UseCompactMode",
                "AutoCheckSelect",
                "Hidden",
                "LaunchTo",
                "FullRowSelect",
                "NoStrCmpLogical",
            ]
        );
        for row in &rows {
            assert_eq!(row.value, "Unavailable");
            assert_eq!(row.status, "Not captured");
        }

        let mut snapshot2 = empty_snapshot();
        snapshot2.values.insert(
            "UnrelatedValue".into(),
            dword_observation("UnrelatedValue", 1),
        );
        let rows2 = advanced_rows(&snapshot2);
        assert_eq!(rows2.len(), 7);
    }

    #[test]
    fn advanced_rows_preserve_dword_values() {
        let mut snapshot = empty_snapshot();
        let values = [0u32, 1, 2, 3, 42, 256, u32::MAX];
        let names = [
            HIDE_FILE_EXT,
            USE_COMPACT_MODE,
            AUTO_CHECK_SELECT,
            HIDDEN,
            LAUNCH_TO,
            FULL_ROW_SELECT,
            NO_STR_CMP_LOGICAL,
        ];
        for (n, v) in names.iter().zip(values.iter()) {
            snapshot
                .values
                .insert((*n).into(), dword_observation(n, *v));
        }
        let rows = advanced_rows(&snapshot);
        let expected = ["0", "1", "2", "3", "42", "256", "4294967295"];
        for (row, exp) in rows.iter().zip(expected.iter()) {
            assert_eq!(row.value, *exp);
            assert_eq!(row.status, "Captured");
        }
    }

    #[test]
    fn advanced_rows_distinguish_missing_and_failed_reads() {
        let mut snapshot = empty_snapshot();
        snapshot.values.insert(
            "HideFileExt".into(),
            ValueReadResult::Failure(ReadError::ValueNotFound {
                name: "HideFileExt".into(),
            }),
        );
        snapshot.values.insert(
            "UseCompactMode".into(),
            ValueReadResult::Failure(ReadError::KeyNotFound {
                path: ADVANCED.into(),
            }),
        );
        snapshot.values.insert(
            "AutoCheckSelect".into(),
            ValueReadResult::Failure(ReadError::AccessDenied {
                path: ADVANCED.into(),
            }),
        );
        snapshot.values.insert(
            "Hidden".into(),
            ValueReadResult::Failure(ReadError::Other("boom".into())),
        );

        let rows = advanced_rows(&snapshot);
        // HideFileExt -> Value not found
        assert_eq!(rows[0].status, "Value not found");
        // UseCompactMode -> Key not found
        assert_eq!(rows[1].status, "Key not found");
        // AutoCheckSelect -> Access denied
        assert_eq!(rows[2].status, "Access denied");
        // Hidden -> Read failed
        assert_eq!(rows[3].status, "Read failed");
        // LaunchTo missing -> Not captured
        assert_eq!(rows[4].status, "Not captured");
        for row in &rows {
            assert_eq!(row.value, "Unavailable");
        }
    }

    #[test]
    fn advanced_rows_distinguish_decoding_failures() {
        let mut snapshot = empty_snapshot();
        let malformed = ValueReadResult::Success(RawRegistryValue {
            key_path: ADVANCED.into(),
            value_name: "HideFileExt".into(),
            root: RootKey::Hkcu,
            raw_type_id: 4,
            raw_bytes: vec![],
            decoded: None,
            decoding_status: DecodingStatus::Malformed {
                reason: "bad".into(),
            },
            source: ObservationSource::HkcuAdvanced,
        });
        let unknown = ValueReadResult::Success(RawRegistryValue {
            key_path: ADVANCED.into(),
            value_name: "UseCompactMode".into(),
            root: RootKey::Hkcu,
            raw_type_id: 4,
            raw_bytes: vec![],
            decoded: None,
            decoding_status: DecodingStatus::UnknownType,
            source: ObservationSource::HkcuAdvanced,
        });
        snapshot.values.insert("HideFileExt".into(), malformed);
        snapshot.values.insert("UseCompactMode".into(), unknown);
        let rows = advanced_rows(&snapshot);
        assert_eq!(rows[0].status, "Malformed data");
        assert_eq!(rows[1].status, "Unknown registry type");
        assert_eq!(rows[0].value, "Unavailable");
        assert_eq!(rows[1].value, "Unavailable");
    }

    #[test]
    fn advanced_rows_reject_unexpected_value_types() {
        let names = [
            HIDE_FILE_EXT,
            USE_COMPACT_MODE,
            AUTO_CHECK_SELECT,
            HIDDEN,
            LAUNCH_TO,
            FULL_ROW_SELECT,
        ];
        // String, Qword, Binary, MultiString, inconsistent raw_type_id, missing decoded
        let mut snapshot = empty_snapshot();
        // String
        snapshot.values.insert(
            "HideFileExt".into(),
            ValueReadResult::Success(RawRegistryValue {
                key_path: ADVANCED.into(),
                value_name: "HideFileExt".into(),
                root: RootKey::Hkcu,
                raw_type_id: 1,
                raw_bytes: vec![],
                decoded: Some(RegistryValue::String("x".into())),
                decoding_status: DecodingStatus::Decoded,
                source: ObservationSource::HkcuAdvanced,
            }),
        );
        // Qword
        snapshot.values.insert(
            "UseCompactMode".into(),
            ValueReadResult::Success(RawRegistryValue {
                key_path: ADVANCED.into(),
                value_name: "UseCompactMode".into(),
                root: RootKey::Hkcu,
                raw_type_id: 11,
                raw_bytes: vec![],
                decoded: Some(RegistryValue::Qword(1)),
                decoding_status: DecodingStatus::Decoded,
                source: ObservationSource::HkcuAdvanced,
            }),
        );
        // Binary
        snapshot.values.insert(
            "AutoCheckSelect".into(),
            ValueReadResult::Success(RawRegistryValue {
                key_path: ADVANCED.into(),
                value_name: "AutoCheckSelect".into(),
                root: RootKey::Hkcu,
                raw_type_id: 3,
                raw_bytes: vec![],
                decoded: Some(RegistryValue::Binary(vec![1])),
                decoding_status: DecodingStatus::Decoded,
                source: ObservationSource::HkcuAdvanced,
            }),
        );
        // MultiString
        snapshot.values.insert(
            "Hidden".into(),
            ValueReadResult::Success(RawRegistryValue {
                key_path: ADVANCED.into(),
                value_name: "Hidden".into(),
                root: RootKey::Hkcu,
                raw_type_id: 7,
                raw_bytes: vec![],
                decoded: Some(RegistryValue::MultiString(vec!["a".into()])),
                decoding_status: DecodingStatus::Decoded,
                source: ObservationSource::HkcuAdvanced,
            }),
        );
        // DWORD inconsistent raw_type_id
        snapshot.values.insert(
            "LaunchTo".into(),
            ValueReadResult::Success(RawRegistryValue {
                key_path: ADVANCED.into(),
                value_name: "LaunchTo".into(),
                root: RootKey::Hkcu,
                raw_type_id: 1,
                raw_bytes: vec![],
                decoded: Some(RegistryValue::Dword(5)),
                decoding_status: DecodingStatus::Decoded,
                source: ObservationSource::HkcuAdvanced,
            }),
        );
        // missing decoded
        snapshot.values.insert(
            "FullRowSelect".into(),
            ValueReadResult::Success(RawRegistryValue {
                key_path: ADVANCED.into(),
                value_name: "FullRowSelect".into(),
                root: RootKey::Hkcu,
                raw_type_id: 4,
                raw_bytes: vec![],
                decoded: None,
                decoding_status: DecodingStatus::Decoded,
                source: ObservationSource::HkcuAdvanced,
            }),
        );

        let rows = advanced_rows(&snapshot);
        for row in rows.iter().take(6) {
            assert_eq!(row.value, "Unavailable");
            assert_eq!(row.status, "Unexpected value type");
        }
    }

    #[test]
    fn advanced_rows_do_not_modify_snapshot_or_count_failures_as_errors() {
        use winview_core::model::InspectionError;
        let mut snapshot = empty_snapshot();
        snapshot
            .values
            .insert("HideFileExt".into(), dword_observation("HideFileExt", 1));
        snapshot.values.insert(
            "UseCompactMode".into(),
            ValueReadResult::Failure(ReadError::ValueNotFound {
                name: "UseCompactMode".into(),
            }),
        );
        snapshot.values.insert(
            "AutoCheckSelect".into(),
            ValueReadResult::Failure(ReadError::AccessDenied {
                path: ADVANCED.into(),
            }),
        );
        snapshot.read_errors.push(InspectionError {
            key_path: ADVANCED.into(),
            root: RootKey::Hkcu,
            operation: "get_value".into(),
            error: "denied".into(),
        });

        let before = count_key_read_errors(&snapshot);
        let rows = advanced_rows(&snapshot);
        let after = count_key_read_errors(&snapshot);
        assert_eq!(before, 1);
        assert_eq!(after, 1);

        assert_eq!(rows[0].status, "Captured");
        assert_eq!(rows[1].status, "Value not found");
        assert_eq!(rows[2].status, "Access denied");
    }
}
