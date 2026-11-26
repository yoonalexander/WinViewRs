// winview-app/src/cli/inspect.rs

use anyhow::Result;
use std::io::{self, Write};
use std::process::ExitCode;
use winview_core::inspection::count_snapshot_read_errors;
use winview_core::model::InspectionSnapshot;
use winview_core::RegistryReader;

use super::write_snapshot_with_status;

pub fn run_inspect() -> Result<ExitCode> {
    let reader = winview_registry::WindowsRegistryReader;

    let snapshot = reader.inspect_all()?;

    let stdout = io::stdout();

    write_snapshot_with_status(&snapshot, stdout.lock(), write_snapshot_summary)
}

pub(super) fn write_snapshot_summary(
    snapshot: &InspectionSnapshot,
    mut writer: impl Write,
) -> Result<()> {
    writeln!(writer, "WinViewRs Inspection Snapshot")?;
    writeln!(writer, "=============================")?;
    writeln!(writer, "Captured at: {}", snapshot.metadata.captured_at)?;
    writeln!(
        writer,
        "Windows Build: {:?}",
        snapshot.metadata.windows_build
    )?;
    writeln!(writer, "Windows UBR: {:?}", snapshot.metadata.windows_ubr)?;
    writeln!(
        writer,
        "Product Name: {:?}",
        snapshot.metadata.windows_product_name
    )?;
    writeln!(
        writer,
        "Detection Method: {:?}",
        snapshot.metadata.detection_method
    )?;
    writeln!(writer, "Status: {:?}", snapshot.status)?;
    writeln!(writer)?;

    writeln!(
        writer,
        "HKCU Folder Types: {} types",
        snapshot.hkcu_folder_types.types.len()
    )?;
    for (guid, ft) in &snapshot.hkcu_folder_types.types {
        writeln!(
            writer,
            "  {} ({})",
            ft.canonical_name.as_deref().unwrap_or("?"),
            guid
        )?;
        for (view_guid, view) in &ft.top_views {
            writeln!(
                writer,
                "    View {}: LVM={:?} Mode={:?} IconSize={:?} GroupBy={:?} SortBy={:?}",
                view_guid,
                view.logical_view_mode,
                view.mode,
                view.icon_size,
                view.group_by,
                view.sort_by_list
            )?;
        }
    }
    writeln!(
        writer,
        "HKLM Folder Types: {} types",
        snapshot.hklm_folder_types.types.len()
    )?;
    writeln!(writer, "Bags: {} top-level keys", snapshot.bags.len())?;
    writeln!(writer, "Streams: {} top-level keys", snapshot.streams.len())?;
    writeln!(
        writer,
        "Advanced: {} value observations",
        snapshot.advanced.values.len()
    )?;
    writeln!(writer, "CLSID: {} top-level keys", snapshot.clsid.len())?;
    writeln!(
        writer,
        "Policies: {} top-level keys",
        snapshot.policies.len()
    )?;
    writeln!(writer, "Errors: {}", count_snapshot_read_errors(snapshot))?;

    writer.flush()?;
    Ok(())
}

pub(crate) fn write_snapshot_json(
    snapshot: &InspectionSnapshot,
    mut writer: impl Write,
) -> Result<()> {
    serde_json::to_writer_pretty(&mut writer, snapshot)?;

    writeln!(writer)?;
    writer.flush()?;

    Ok(())
}

pub fn run_dump_json() -> Result<ExitCode> {
    let reader = winview_registry::WindowsRegistryReader;

    let snapshot = reader.inspect_all()?;

    let stdout = io::stdout();

    write_snapshot_with_status(&snapshot, stdout.lock(), write_snapshot_json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use winview_core::model::{
        DecodingStatus, FolderTypeSnapshot, InspectionError, InspectionStatus, KeySnapshot,
        ObservationSource, RawRegistryValue, ReadError, ValueReadResult,
    };
    use winview_core::registry::keys::RootKey;

    #[test]
    fn json_writer_preserves_inspection_statuses() {
        let statuses = [
            InspectionStatus::Complete,
            InspectionStatus::Partial {
                reason: "injected partial result".into(),
            },
            InspectionStatus::Failed {
                error: "injected failure".into(),
            },
        ];

        for status in statuses {
            let snapshot = InspectionSnapshot {
                status: status.clone(),
                ..Default::default()
            };

            let mut bytes = Vec::new();

            write_snapshot_json(&snapshot, &mut bytes).unwrap();

            let parsed: InspectionSnapshot = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(parsed.status, status);

            assert_eq!(bytes.last(), Some(&b'\n'));
        }
    }

    #[test]
    fn json_writer_preserves_raw_values_and_failures() {
        let mut snapshot = InspectionSnapshot::default();

        let expected = RawRegistryValue {
            key_path: "Software\\Test".into(),
            value_name: "UnknownValue".into(),
            root: RootKey::Hkcu,
            raw_type_id: 999,
            raw_bytes: vec![0, 128, 255, 42],
            decoded: None,
            decoding_status: DecodingStatus::UnknownType,
            source: ObservationSource::HkcuAdvanced,
        };

        snapshot.advanced.values.insert(
            "UnknownValue".into(),
            ValueReadResult::Success(expected.clone()),
        );

        snapshot.advanced.values.insert(
            "MissingValue".into(),
            ValueReadResult::Failure(ReadError::ValueNotFound {
                name: "MissingValue".into(),
            }),
        );

        let mut bytes = Vec::new();

        write_snapshot_json(&snapshot, &mut bytes).unwrap();

        let parsed: InspectionSnapshot = serde_json::from_slice(&bytes).unwrap();

        match parsed.advanced.values.get("UnknownValue") {
            Some(ValueReadResult::Success(raw)) => {
                assert_eq!(raw, &expected);
            }
            _ => panic!("Raw observation was not preserved"),
        }

        assert!(matches!(
            parsed.advanced.values.get("MissingValue"),
            Some(ValueReadResult::Failure(ReadError::ValueNotFound { name })) if name == "MissingValue"
        ));
    }

    #[test]
    fn json_writer_propagates_io_errors() {
        struct FailingWriter;

        impl Write for FailingWriter {
            fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("injected write failure"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let snapshot = InspectionSnapshot::default();

        let error = write_snapshot_json(&snapshot, FailingWriter)
            .expect_err("Writer failure must propagate");

        assert!(error.to_string().contains("injected write failure"));
    }

    #[test]
    fn human_summary_counts_all_recorded_errors() {
        let mut snapshot = InspectionSnapshot::default();

        let error = |path: &str| InspectionError {
            key_path: path.into(),
            root: RootKey::Hkcu,
            operation: "open_key".into(),
            error: "injected failure".into(),
        };

        snapshot.errors.push(error("global"));

        snapshot
            .hkcu_folder_types
            .read_errors
            .push(error("HKCU FolderTypes"));

        snapshot
            .hklm_folder_types
            .read_errors
            .push(InspectionError {
                root: RootKey::Hklm,
                ..error("HKLM FolderTypes")
            });

        let mut hkcu_folder = FolderTypeSnapshot::default();

        hkcu_folder.read_errors.push(error("HKCU folder"));

        snapshot
            .hkcu_folder_types
            .types
            .insert("folder1".into(), hkcu_folder);

        let mut hklm_folder = FolderTypeSnapshot::default();

        hklm_folder.read_errors.push(InspectionError {
            root: RootKey::Hklm,
            ..error("HKLM folder")
        });

        snapshot
            .hklm_folder_types
            .types
            .insert("folder2".into(), hklm_folder);

        snapshot.advanced.read_errors.push(error("Advanced"));

        for domain in [
            &mut snapshot.bags,
            &mut snapshot.streams,
            &mut snapshot.clsid,
            &mut snapshot.policies,
        ] {
            let mut root = KeySnapshot::default();
            let mut child = KeySnapshot::default();

            child.read_errors.push(error("root\\child"));

            root.subkeys.insert("root\\child".into(), child);

            domain.insert("root".into(), root);
        }

        assert_eq!(count_snapshot_read_errors(&snapshot), 10);

        let mut bytes = Vec::new();

        write_snapshot_summary(&snapshot, &mut bytes).unwrap();

        let output = String::from_utf8(bytes).unwrap();

        assert!(output.lines().any(|line| line == "Errors: 10"));

        assert!(output.lines().any(|line| line == "Bags: 1 top-level keys"));
    }

    #[test]
    fn human_summary_does_not_double_count_failed_values() {
        let mut snapshot = InspectionSnapshot::default();

        snapshot.advanced.values.insert(
            "MissingValue".into(),
            ValueReadResult::Failure(ReadError::ValueNotFound {
                name: "MissingValue".into(),
            }),
        );

        assert_eq!(count_snapshot_read_errors(&snapshot), 0);

        snapshot.advanced.values.insert(
            "DeniedValue".into(),
            ValueReadResult::Failure(ReadError::AccessDenied {
                path: "Advanced".into(),
            }),
        );

        snapshot.advanced.read_errors.push(InspectionError {
            key_path: "Advanced".into(),
            root: RootKey::Hkcu,
            operation: "get_value".into(),
            error: "DeniedValue: Access denied".into(),
        });

        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        let mut bytes = Vec::new();

        write_snapshot_summary(&snapshot, &mut bytes).unwrap();

        let output = String::from_utf8(bytes).unwrap();

        assert!(output.lines().any(|line| line == "Errors: 1"));

        assert!(output
            .lines()
            .any(|line| line == "Advanced: 2 value observations"));
    }

    #[test]
    fn human_summary_preserves_inspection_status() {
        let statuses = [
            InspectionStatus::Complete,
            InspectionStatus::Partial {
                reason: "injected partial result".into(),
            },
            InspectionStatus::Failed {
                error: "injected failure".into(),
            },
        ];

        for status in statuses {
            let snapshot = InspectionSnapshot {
                status: status.clone(),
                ..Default::default()
            };

            let mut bytes = Vec::new();

            write_snapshot_summary(&snapshot, &mut bytes).unwrap();

            let output = String::from_utf8(bytes).unwrap();

            let expected = format!("Status: {:?}", status);

            assert!(output.lines().any(|line| line == expected));

            assert!(output.contains("WinViewRs Inspection Snapshot"));
        }
    }

    #[test]
    fn human_summary_propagates_io_errors() {
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
            let error = write_snapshot_summary(&snapshot, FailingWriter { fail_on_flush })
                .expect_err("I/O failure must propagate");

            assert!(error.to_string().contains(expected));
        }
    }
}
