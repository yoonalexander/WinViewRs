// winview-app/src/cli/mod.rs

pub mod inspect;
pub mod output;

use anyhow::Result;
use std::io::Write;
use std::process::ExitCode;
use winview_core::model::{InspectionSnapshot, InspectionStatus};

fn inspection_exit_code(status: &InspectionStatus) -> ExitCode {
    match status {
        InspectionStatus::Complete => ExitCode::SUCCESS,
        InspectionStatus::Partial { .. } => ExitCode::from(3),
        InspectionStatus::Failed { .. } => ExitCode::FAILURE,
    }
}

fn write_snapshot_with_status<W: Write>(
    snapshot: &InspectionSnapshot,
    writer: W,
    render: impl FnOnce(&InspectionSnapshot, W) -> Result<()>,
) -> Result<ExitCode> {
    render(snapshot, writer)?;

    Ok(inspection_exit_code(&snapshot.status))
}

#[cfg(test)]
mod tests {
    use super::inspect::{write_snapshot_json, write_snapshot_summary};
    use super::output::write_snapshot_toml;
    use super::*;
    use std::io;

    #[test]
    fn all_renderers_return_consistent_inspection_exit_codes() {
        let cases = [
            (InspectionStatus::Complete, 0u8),
            (
                InspectionStatus::Partial {
                    reason: "injected partial result".into(),
                },
                3,
            ),
            (
                InspectionStatus::Failed {
                    error: "injected failure".into(),
                },
                1,
            ),
        ];

        for (status, code) in cases {
            let snapshot = InspectionSnapshot {
                status: status.clone(),
                ..Default::default()
            };

            let mut summary = Vec::new();
            let mut json = Vec::new();
            let mut toml = Vec::new();

            let summary_code =
                write_snapshot_with_status(&snapshot, &mut summary, write_snapshot_summary)
                    .unwrap();

            let json_code =
                write_snapshot_with_status(&snapshot, &mut json, write_snapshot_json).unwrap();

            let toml_code =
                write_snapshot_with_status(&snapshot, &mut toml, write_snapshot_toml).unwrap();

            assert_eq!(summary_code, ExitCode::from(code));

            assert_eq!(json_code, ExitCode::from(code));

            assert_eq!(toml_code, ExitCode::from(code));

            let summary = String::from_utf8(summary).unwrap();

            assert!(summary.contains(&format!("Status: {:?}", status)));

            let parsed_json: InspectionSnapshot = serde_json::from_slice(&json).unwrap();

            assert_eq!(parsed_json.status, status);

            let toml_text = String::from_utf8(toml).unwrap();

            let parsed_toml: toml::Value = toml::from_str(&toml_text).unwrap();

            let expected_state = match status {
                InspectionStatus::Complete => "Complete",
                InspectionStatus::Partial { .. } => "Partial",
                InspectionStatus::Failed { .. } => "Failed",
            };

            assert_eq!(
                parsed_toml["status"]["state"].as_str(),
                Some(expected_state)
            );
        }
    }

    #[derive(Clone, Copy)]
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

    fn assert_all_renderers_fail(
        snapshot: &InspectionSnapshot,
        writer: FailingWriter,
        expected: &str,
    ) {
        let results = [
            write_snapshot_with_status(snapshot, writer, write_snapshot_summary),
            write_snapshot_with_status(snapshot, writer, write_snapshot_json),
            write_snapshot_with_status(snapshot, writer, write_snapshot_toml),
        ];

        for result in results {
            let error = result.expect_err("Output failure must propagate");

            assert!(error.to_string().contains(expected));
        }
    }

    #[test]
    fn write_failure_overrides_inspection_exit_code() {
        let snapshot = InspectionSnapshot {
            status: InspectionStatus::Partial {
                reason: "injected partial result".into(),
            },
            ..Default::default()
        };

        assert_all_renderers_fail(
            &snapshot,
            FailingWriter {
                fail_on_flush: false,
            },
            "injected write failure",
        );
    }

    #[test]
    fn flush_failure_overrides_inspection_exit_code() {
        let snapshot = InspectionSnapshot {
            status: InspectionStatus::Complete,
            ..Default::default()
        };

        assert_all_renderers_fail(
            &snapshot,
            FailingWriter {
                fail_on_flush: true,
            },
            "injected flush failure",
        );
    }

    #[test]
    fn toml_serialization_failure_overrides_inspection_exit_code() {
        use winview_core::model::{
            DecodingStatus, ObservationSource, RawRegistryValue, RegistryValue, ValueReadResult,
        };
        use winview_core::registry::keys::RootKey;

        let mut snapshot = InspectionSnapshot {
            status: InspectionStatus::Partial {
                reason: "injected partial result".into(),
            },
            ..Default::default()
        };

        snapshot.advanced.values.insert(
            "QwordValue".into(),
            ValueReadResult::Success(RawRegistryValue {
                key_path: "Software\\Test".into(),
                value_name: "QwordValue".into(),
                root: RootKey::Hkcu,
                raw_type_id: 11,
                raw_bytes: u64::MAX.to_le_bytes().to_vec(),
                decoded: Some(RegistryValue::Qword(u64::MAX)),
                decoding_status: DecodingStatus::Decoded,
                source: ObservationSource::HkcuAdvanced,
            }),
        );

        let mut bytes = Vec::new();

        let error = write_snapshot_with_status(&snapshot, &mut bytes, write_snapshot_toml)
            .expect_err("Serialization failure must propagate");

        assert!(bytes.is_empty());

        assert!(error.to_string().contains("use dump --format json instead"));
    }
}
