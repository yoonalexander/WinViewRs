// winview-app/src/gui/overview.rs

use std::collections::BTreeMap;

use eframe::egui;

use winview_core::inspection::{
    count_folder_types_read_errors, count_key_read_errors, snapshot_has_successful_observations,
};

use winview_core::model::{DetectionMethod, InspectionSnapshot, KeySnapshot, SnapshotMetadata};

pub(super) struct DomainOverview {
    pub(super) name: &'static str,
    pub(super) count: usize,
    pub(super) unit: &'static str,
    pub(super) errors: usize,
}

pub(super) struct InspectionOverview {
    pub(super) metadata: [(&'static str, String); 5],

    pub(super) domains: [DomainOverview; 7],

    pub(super) global_errors: usize,

    pub(super) has_observations: bool,
}

fn optional_text(value: Option<&str>) -> String {
    value
        .filter(|text| !text.trim().is_empty())
        .unwrap_or("Unavailable")
        .to_string()
}

fn optional_number(value: Option<u32>) -> String {
    value
        .map(|number| number.to_string())
        .unwrap_or_else(|| "Unavailable".into())
}

fn metadata_rows(metadata: &SnapshotMetadata) -> [(&'static str, String); 5] {
    let method = match metadata.detection_method {
        DetectionMethod::RtlGetVersion => "RtlGetVersion",
        DetectionMethod::RegistryFallback => "Registry fallback",
        DetectionMethod::Failed => "Detection failed",
    };

    [
        (
            "Product name",
            optional_text(metadata.windows_product_name.as_deref()),
        ),
        ("Windows build", optional_number(metadata.windows_build)),
        ("Windows UBR", optional_number(metadata.windows_ubr)),
        ("Detection method", method.to_string()),
        ("Captured at", optional_text(Some(&metadata.captured_at))),
    ]
}

fn recursive_domain(name: &'static str, keys: &BTreeMap<String, KeySnapshot>) -> DomainOverview {
    DomainOverview {
        name,
        count: keys.len(),
        unit: "root records",
        errors: keys.values().map(count_key_read_errors).sum(),
    }
}

impl InspectionOverview {
    pub(super) fn from_snapshot(snapshot: &InspectionSnapshot) -> Self {
        Self {
            metadata: metadata_rows(&snapshot.metadata),

            domains: [
                DomainOverview {
                    name: "HKCU FolderTypes",
                    count: snapshot.hkcu_folder_types.types.len(),
                    unit: "folder types",
                    errors: count_folder_types_read_errors(&snapshot.hkcu_folder_types),
                },
                DomainOverview {
                    name: "HKLM FolderTypes",
                    count: snapshot.hklm_folder_types.types.len(),
                    unit: "folder types",
                    errors: count_folder_types_read_errors(&snapshot.hklm_folder_types),
                },
                recursive_domain("Bags", &snapshot.bags),
                recursive_domain("Streams", &snapshot.streams),
                DomainOverview {
                    name: "Advanced",
                    count: snapshot.advanced.values.len(),
                    unit: "value records",
                    errors: count_key_read_errors(&snapshot.advanced),
                },
                recursive_domain("CLSID", &snapshot.clsid),
                recursive_domain("Policies", &snapshot.policies),
            ],

            global_errors: snapshot.errors.len(),

            has_observations: snapshot_has_successful_observations(snapshot),
        }
    }
}

pub(super) fn show_overview(ui: &mut egui::Ui, overview: &InspectionOverview) {
    ui.separator();

    if !overview.has_observations {
        ui.label("No successful registry observations were captured.");
    }

    ui.heading("Windows information");

    egui::Grid::new("inspection_metadata")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            for (label, value) in &overview.metadata {
                ui.label(*label);
                ui.label(value.as_str());
                ui.end_row();
            }
        });

    ui.add_space(12.0);

    ui.heading("Inspection domains");

    egui::Grid::new("inspection_domains")
        .num_columns(3)
        .striped(true)
        .show(ui, |ui| {
            ui.strong("Domain");
            ui.strong("Captured entries");
            ui.strong("Errors");
            ui.end_row();

            for domain in &overview.domains {
                ui.label(domain.name);
                ui.label(format!("{} {}", domain.count, domain.unit));
                ui.label(domain.errors.to_string());
                ui.end_row();
            }
        });

    ui.label(format!(
        "Global inspection errors: {}",
        overview.global_errors,
    ));

    ui.add_space(8.0);

    ui.small(
        "Counts describe captured snapshot records. \
         They may include failed reads, overlapping \
         registry roots, or incomplete data.",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    use winview_core::inspection::count_snapshot_read_errors;

    use winview_core::model::{FolderTypeSnapshot, InspectionError, ReadError, ValueReadResult};

    use winview_core::RootKey;

    fn error(path: &str) -> InspectionError {
        InspectionError {
            key_path: path.into(),
            root: RootKey::Hkcu,
            operation: "get_value".into(),
            error: "injected failure".into(),
        }
    }

    #[test]
    fn overview_formats_recorded_metadata() {
        let metadata = SnapshotMetadata {
            captured_at: "2026-01-01T12:00:00Z".into(),
            windows_build: Some(26100),
            windows_ubr: Some(0),
            windows_product_name: Some("Synthetic Windows".into()),
            detection_method: DetectionMethod::RtlGetVersion,
        };

        let rows = metadata_rows(&metadata);

        assert_eq!(rows[0].1, "Synthetic Windows");
        assert_eq!(rows[1].1, "26100");
        assert_eq!(rows[2].1, "0");
        assert_eq!(rows[3].1, "RtlGetVersion");
        assert_eq!(rows[4].1, "2026-01-01T12:00:00Z");
    }

    #[test]
    fn overview_handles_missing_metadata() {
        let mut metadata = SnapshotMetadata::default();

        let rows = metadata_rows(&metadata);

        assert_eq!(rows[0].1, "Unavailable");
        assert_eq!(rows[1].1, "Unavailable");
        assert_eq!(rows[2].1, "Unavailable");
        assert_eq!(rows[3].1, "Detection failed");
        assert_eq!(rows[4].1, "Unavailable");

        metadata.detection_method = DetectionMethod::RegistryFallback;
        metadata.windows_product_name = Some("   ".into());

        let rows = metadata_rows(&metadata);

        assert_eq!(rows[0].1, "Unavailable");
        assert_eq!(rows[3].1, "Registry fallback");
    }

    #[test]
    fn overview_counts_all_domains_and_nested_errors() {
        let mut snapshot = InspectionSnapshot::default();

        snapshot.errors.push(error("global"));

        for folder_types in [
            &mut snapshot.hkcu_folder_types,
            &mut snapshot.hklm_folder_types,
        ] {
            folder_types.read_errors.push(error("FolderTypes"));

            let mut folder = FolderTypeSnapshot::default();
            folder.read_errors.push(error("FolderTypes\\GUID"));
            folder_types.types.insert("GUID".into(), folder);
        }

        for domain in [
            &mut snapshot.bags,
            &mut snapshot.streams,
            &mut snapshot.clsid,
            &mut snapshot.policies,
        ] {
            let mut root = KeySnapshot::default();
            root.read_errors.push(error("root"));

            let mut child = KeySnapshot::default();
            child.read_errors.push(error("root\\child"));

            root.subkeys.insert("child".into(), child);
            domain.insert("root".into(), root);
        }

        snapshot.advanced.values.insert(
            "Missing".into(),
            ValueReadResult::Failure(ReadError::ValueNotFound {
                name: "Missing".into(),
            }),
        );

        snapshot.advanced.read_errors.push(error("Advanced"));

        let overview = InspectionOverview::from_snapshot(&snapshot);

        let names: Vec<_> = overview.domains.iter().map(|domain| domain.name).collect();

        assert_eq!(
            names,
            vec![
                "HKCU FolderTypes",
                "HKLM FolderTypes",
                "Bags",
                "Streams",
                "Advanced",
                "CLSID",
                "Policies",
            ]
        );

        for domain in &overview.domains {
            assert_eq!(domain.count, 1);
        }

        let counts: Vec<_> = overview
            .domains
            .iter()
            .map(|domain| domain.errors)
            .collect();

        assert_eq!(counts, vec![2, 2, 2, 2, 1, 2, 2]);

        assert_eq!(overview.global_errors, 1);

        let total = overview.global_errors
            + overview
                .domains
                .iter()
                .map(|domain| domain.errors)
                .sum::<usize>();

        assert_eq!(total, 14);

        assert_eq!(total, count_snapshot_read_errors(&snapshot));

        assert!(!overview.has_observations);
    }

    #[test]
    fn overview_distinguishes_empty_and_captured_data() {
        let mut snapshot = InspectionSnapshot::default();

        let overview = InspectionOverview::from_snapshot(&snapshot);

        assert!(!overview.has_observations);
        assert_eq!(overview.global_errors, 0);
        assert!(overview
            .domains
            .iter()
            .all(|domain| { domain.count == 0 && domain.errors == 0 }));

        snapshot.hkcu_folder_types.types.insert(
            "GUID".into(),
            FolderTypeSnapshot {
                guid: "GUID".into(),
                canonical_name: Some("Documents".into()),
                ..Default::default()
            },
        );

        let overview = InspectionOverview::from_snapshot(&snapshot);

        assert!(overview.has_observations);
        assert_eq!(overview.domains[0].count, 1);
    }
}
