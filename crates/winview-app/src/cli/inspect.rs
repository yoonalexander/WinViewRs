// winview-app/src/cli/inspect.rs

use anyhow::Result;
use std::io::{self, Write};
use winview_core::model::InspectionSnapshot;
use winview_core::RegistryReader;

pub fn run_inspect() -> Result<()> {
    let reader = winview_registry::WindowsRegistryReader;
    let snapshot = reader.inspect_all()?;

    // Human-readable table output
    println!("WinViewRs Inspection Snapshot");
    println!("=============================");
    println!("Captured at: {}", snapshot.metadata.captured_at);
    println!("Windows Build: {:?}", snapshot.metadata.windows_build);
    println!("Windows UBR: {:?}", snapshot.metadata.windows_ubr);
    println!("Product Name: {:?}", snapshot.metadata.windows_product_name);
    println!("Detection Method: {:?}", snapshot.metadata.detection_method);
    println!("Status: {:?}", snapshot.status);
    println!();

    println!(
        "HKCU Folder Types: {} types",
        snapshot.hkcu_folder_types.types.len()
    );
    for (guid, ft) in &snapshot.hkcu_folder_types.types {
        println!(
            "  {} ({})",
            ft.canonical_name.as_deref().unwrap_or("?"),
            guid
        );
        for (view_guid, view) in &ft.top_views {
            println!(
                "    View {}: LVM={:?} Mode={:?} IconSize={:?} GroupBy={:?} SortBy={:?}",
                view_guid,
                view.logical_view_mode,
                view.mode,
                view.icon_size,
                view.group_by,
                view.sort_by_list
            );
        }
    }
    println!(
        "HKLM Folder Types: {} types",
        snapshot.hklm_folder_types.types.len()
    );
    println!("Bags: {} keys", snapshot.bags.len());
    println!("Streams: {} keys", snapshot.streams.len());
    println!("Advanced: {} values", snapshot.advanced.values.len());
    println!("CLSID: {} keys", snapshot.clsid.len());
    println!("Policies: {} keys", snapshot.policies.len());
    println!("Errors: {}", snapshot.errors.len());

    Ok(())
}

fn write_snapshot_json(snapshot: &InspectionSnapshot, mut writer: impl Write) -> Result<()> {
    serde_json::to_writer_pretty(&mut writer, snapshot)?;

    writeln!(writer)?;
    writer.flush()?;

    Ok(())
}

pub fn run_dump_json() -> Result<()> {
    let reader = winview_registry::WindowsRegistryReader;

    let snapshot = reader.inspect_all()?;

    let stdout = io::stdout();

    write_snapshot_json(&snapshot, stdout.lock())
}

#[cfg(test)]
mod tests {
    use super::*;
    use winview_core::model::{
        DecodingStatus, InspectionStatus, ObservationSource, RawRegistryValue, ReadError,
        ValueReadResult,
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
}
