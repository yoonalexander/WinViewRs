// winview-app/src/cli/output.rs

use anyhow::{Context, Result};
use std::io::{self, Write};
use winview_core::model::InspectionSnapshot;
use winview_core::RegistryReader;

pub fn run_export_toml() -> Result<()> {
    let reader = winview_registry::WindowsRegistryReader;

    let snapshot = reader.inspect_all()?;

    let stdout = io::stdout();

    write_snapshot_toml(&snapshot, stdout.lock())
}

fn write_snapshot_toml(snapshot: &InspectionSnapshot, mut writer: impl Write) -> Result<()> {
    let serialized = toml::to_string_pretty(snapshot)
        .context("TOML export could not represent the snapshot; use dump --format json instead")?;

    writer.write_all(serialized.as_bytes())?;

    if !serialized.ends_with('\n') {
        writeln!(writer)?;
    }

    writer.flush()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use winview_core::model::{
        DecodingStatus, InspectionError, InspectionStatus, KeySnapshot, ObservationSource,
        RawRegistryValue, ReadError, RegistryValue, ValueReadResult,
    };
    use winview_core::registry::keys::RootKey;

    #[test]
    fn toml_writer_preserves_inspection_statuses() {
        let cases = [
            (InspectionStatus::Complete, "Complete", None),
            (
                InspectionStatus::Partial {
                    reason: "injected partial result".into(),
                },
                "Partial",
                Some(("reason", "injected partial result")),
            ),
            (
                InspectionStatus::Failed {
                    error: "injected failure".into(),
                },
                "Failed",
                Some(("error", "injected failure")),
            ),
        ];

        for (status, expected_state, detail) in cases {
            let snapshot = InspectionSnapshot {
                status,
                ..Default::default()
            };

            let mut bytes = Vec::new();

            write_snapshot_toml(&snapshot, &mut bytes).unwrap();

            let output = String::from_utf8(bytes).unwrap();

            let parsed: toml::Value = toml::from_str(&output).unwrap();

            assert_eq!(parsed["status"]["state"].as_str(), Some(expected_state));

            if let Some((field, expected)) = detail {
                assert_eq!(parsed["status"]["detail"][field].as_str(), Some(expected));
            }

            assert!(output.ends_with('\n'));
        }
    }

    #[test]
    fn toml_writer_preserves_raw_values_and_failures() {
        let mut snapshot = InspectionSnapshot::default();

        snapshot.advanced.values.insert(
            "UnknownValue".into(),
            ValueReadResult::Success(RawRegistryValue {
                key_path: "Software\\Test".into(),
                value_name: "UnknownValue".into(),
                root: RootKey::Hkcu,
                raw_type_id: 999,
                raw_bytes: vec![0, 128, 255, 42],
                decoded: None,
                decoding_status: DecodingStatus::UnknownType,
                source: ObservationSource::HkcuAdvanced,
            }),
        );

        snapshot.advanced.values.insert(
            "MissingValue".into(),
            ValueReadResult::Failure(ReadError::ValueNotFound {
                name: "MissingValue".into(),
            }),
        );

        let mut bytes = Vec::new();

        write_snapshot_toml(&snapshot, &mut bytes).unwrap();

        let output = String::from_utf8(bytes).unwrap();

        let parsed: toml::Value = toml::from_str(&output).unwrap();

        let values = &parsed["advanced"]["values"];

        let unknown = &values["UnknownValue"];

        assert_eq!(unknown["raw_type_id"].as_integer(), Some(999));

        let raw: Vec<i64> = unknown["raw_bytes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_integer().unwrap())
            .collect();

        assert_eq!(raw, vec![0, 128, 255, 42]);

        assert_eq!(
            unknown["decoding_status"]["status"].as_str(),
            Some("UnknownType")
        );

        assert!(unknown.get("decoded").is_none());

        let missing = &values["MissingValue"];

        assert_eq!(missing["kind"].as_str(), Some("ValueNotFound"));

        assert_eq!(missing["detail"]["name"].as_str(), Some("MissingValue"));
    }

    #[test]
    fn toml_writer_preserves_nested_registry_errors() {
        let mut snapshot = InspectionSnapshot::default();

        let mut root = KeySnapshot::default();
        let mut child = KeySnapshot::default();

        child.read_errors.push(InspectionError {
            key_path: r"root\child".into(),
            root: RootKey::Hkcu,
            operation: "open_key".into(),
            error: "injected failure".into(),
        });

        root.subkeys.insert(r"root\child".into(), child);

        snapshot.bags.insert("root".into(), root);

        let mut bytes = Vec::new();

        write_snapshot_toml(&snapshot, &mut bytes).unwrap();

        let output = String::from_utf8(bytes).unwrap();

        let parsed: toml::Value = toml::from_str(&output).unwrap();

        let errors = parsed["bags"]["root"]["subkeys"][r"root\child"]["read_errors"]
            .as_array()
            .unwrap();

        assert_eq!(errors.len(), 1);

        assert_eq!(errors[0]["key_path"].as_str(), Some(r"root\child"));

        assert_eq!(errors[0]["operation"].as_str(), Some("open_key"));

        assert_eq!(errors[0]["error"].as_str(), Some("injected failure"));
    }

    #[test]
    fn toml_writer_handles_qword_range_limits() {
        for value in [i64::MAX as u64, i64::MAX as u64 + 1, u64::MAX] {
            let mut snapshot = InspectionSnapshot::default();

            snapshot.advanced.values.insert(
                "QwordValue".into(),
                ValueReadResult::Success(RawRegistryValue {
                    key_path: "Software\\Test".into(),
                    value_name: "QwordValue".into(),
                    root: RootKey::Hkcu,
                    raw_type_id: 11,
                    raw_bytes: value.to_le_bytes().to_vec(),
                    decoded: Some(RegistryValue::Qword(value)),
                    decoding_status: DecodingStatus::Decoded,
                    source: ObservationSource::HkcuAdvanced,
                }),
            );

            let mut bytes = Vec::new();

            let result = write_snapshot_toml(&snapshot, &mut bytes);

            if value <= i64::MAX as u64 {
                result.unwrap();

                let output = String::from_utf8(bytes).unwrap();

                let parsed: toml::Value = toml::from_str(&output).unwrap();

                assert_eq!(
                    parsed["advanced"]["values"]["QwordValue"]["decoded"]["data"].as_integer(),
                    Some(value as i64)
                );
            } else {
                let error = result.expect_err("Out-of-range TOML integer must fail");

                assert!(bytes.is_empty());

                assert!(error.to_string().contains("use dump --format json instead"));
            }
        }
    }

    #[test]
    fn toml_writer_propagates_io_errors() {
        struct FailingWriter {
            fail_on_flush: bool,
        }

        impl Write for FailingWriter {
            fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
                if self.fail_on_flush {
                    Ok(buffer.len())
                } else {
                    Err(io::Error::other("injected write failure"))
                }
            }

            fn flush(&mut self) -> io::Result<()> {
                Err(io::Error::other("injected flush failure"))
            }
        }

        let snapshot = InspectionSnapshot::default();

        for (fail_on_flush, expected) in [
            (false, "injected write failure"),
            (true, "injected flush failure"),
        ] {
            let error = write_snapshot_toml(&snapshot, FailingWriter { fail_on_flush })
                .expect_err("I/O failure must propagate");

            assert!(error.to_string().contains(expected));
        }
    }
}
