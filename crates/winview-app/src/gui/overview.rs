// winview-app/src/gui/overview.rs

use std::collections::BTreeMap;

use eframe::egui;

use winview_core::inspection::{
    count_folder_types_read_errors, count_key_read_errors, snapshot_has_successful_observations,
};

use winview_core::model::{DetectionMethod, InspectionSnapshot, KeySnapshot, SnapshotMetadata};

use super::style::{helpers, StatusKind};

#[derive(Clone)]
pub(super) struct DomainOverview {
    pub(super) name: &'static str,
    pub(super) count: usize,
    pub(super) unit: &'static str,
    pub(super) errors: usize,
}

#[derive(Clone)]
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
            "Registry product name",
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

/// Windows family classification for presentation only.
/// Does not modify captured metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsFamily {
    Windows11,
    Windows10,
    WindowsServer,
    Unknown,
}

/// Recognized Windows 11 client builds for conservative classification.
const WINDOWS_11_CLIENT_BUILDS: &[u32] = &[22000, 22621, 22631, 26100, 26200];

/// Determines the Windows family for display purposes only.
/// Returns the raw product name unchanged when uncertain.
fn infer_windows_family(
    build: Option<u32>,
    product_name: Option<&str>,
) -> (WindowsFamily, Option<&'static str>) {
    let Some(build) = build else {
        return (WindowsFamily::Unknown, None);
    };

    let is_windows_11_build = WINDOWS_11_CLIENT_BUILDS.contains(&build);

    let product_lower = product_name.map(|s| s.to_lowercase());
    let is_server = product_lower
        .as_deref()
        .map_or(false, |s| s.contains("server"));
    let is_windows_10 = product_lower
        .as_deref()
        .map_or(false, |s| s.contains("windows 10"));
    let is_windows_11_product = product_lower
        .as_deref()
        .map_or(false, |s| s.contains("windows 11"));

    // Conservative classification: only recognize known client builds
    if is_windows_11_build && !is_server {
        (WindowsFamily::Windows11, Some("Windows 11"))
    } else if is_windows_10 && !is_server {
        (WindowsFamily::Windows10, Some("Windows 10"))
    } else if is_server {
        (WindowsFamily::WindowsServer, Some("Windows Server"))
    } else if is_windows_11_product && !is_server {
        // Product name says Windows 11 but build not in known list - uncertain
        (WindowsFamily::Unknown, None)
    } else {
        (WindowsFamily::Unknown, None)
    }
}

pub(super) fn show_overview(ui: &mut egui::Ui, overview: &InspectionOverview) {
    let colors = super::style::ThemeColors::from_style(&ui.style());

    // Summary cards
    ui.horizontal_wrapped(|ui| {
        ui.set_min_width(200.0);

        // Inspection status card
        let status_text = match overview.metadata[3].1.as_str() {
            "RtlGetVersion" | "Registry fallback" => "Complete",
            _ => "Failed",
        };
        let status_kind = if status_text == "Complete" {
            StatusKind::Success
        } else {
            StatusKind::Error
        };
        helpers::summary_card(
            ui,
            "Inspection status",
            status_text,
            Some(&overview.metadata[3].1),
        );

        // Total errors card
        let error_kind = if overview.global_errors > 0 {
            StatusKind::Warning
        } else {
            StatusKind::Success
        };
        helpers::summary_card(
            ui,
            "Recorded errors",
            &overview.global_errors.to_string(),
            Some("Global inspection errors"),
        );

        // Folder types card
        let total_folders = overview.domains[0].count + overview.domains[1].count;
        helpers::summary_card(
            ui,
            "Folder types captured",
            &total_folders.to_string(),
            Some(&format!(
                "HKCU: {} | HKLM: {}",
                overview.domains[0].count, overview.domains[1].count
            )),
        );
    });

    ui.add_space(16.0);

    // Windows information section
    helpers::section_heading(ui, "Windows information");

    // Windows family inference
    let build = overview.metadata[1].1.parse::<u32>().ok();
    let product_name = if overview.metadata[0].1 != "Unavailable" {
        Some(overview.metadata[0].1.as_str())
    } else {
        None
    };
    let (family, family_label) = infer_windows_family(build, product_name);

    if let Some(label) = family_label {
        ui.horizontal(|ui| {
            ui.strong("Windows family:");
            ui.add_space(8.0);
            helpers::status_badge(
                ui,
                label,
                match family {
                    WindowsFamily::Windows11 => StatusKind::Success,
                    _ => StatusKind::Info,
                },
            );
            if family == WindowsFamily::Windows11
                && product_name.map_or(false, |p| p.to_lowercase().contains("windows 10"))
            {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("(Registry product name differs)")
                        .size(12.0)
                        .italics()
                        .color(colors.text_secondary),
                );
            }
        });
        ui.add_space(8.0);
    }

    // Metadata table
    egui::Grid::new("inspection_metadata")
        .num_columns(2)
        .striped(true)
        .min_col_width(180.0)
        .show(ui, |ui| {
            for (label, value) in &overview.metadata {
                ui.label(
                    egui::RichText::new(*label)
                        .size(13.0)
                        .color(colors.text_secondary),
                );
                ui.label(
                    egui::RichText::new(value.as_str())
                        .size(13.0)
                        .color(colors.text),
                );
                ui.end_row();
            }
        });

    ui.add_space(16.0);

    // Inspection domains table
    helpers::section_heading(ui, "Inspection domains");

    egui::Grid::new("inspection_domains")
        .num_columns(3)
        .striped(true)
        .min_col_width(120.0)
        .show(ui, |ui| {
            ui.strong("Domain");
            ui.strong("Captured entries");
            ui.strong("Errors");
            ui.end_row();

            for domain in &overview.domains {
                ui.label(
                    egui::RichText::new(domain.name)
                        .size(13.0)
                        .color(colors.text),
                );
                ui.label(
                    egui::RichText::new(format!("{} {}", domain.count, domain.unit))
                        .size(13.0)
                        .color(colors.text),
                );
                let error_color = if domain.errors > 0 {
                    colors.warning
                } else {
                    colors.success
                };
                ui.label(
                    egui::RichText::new(domain.errors.to_string())
                        .size(13.0)
                        .color(error_color)
                        .strong(),
                );
                ui.end_row();
            }
        });

    ui.add_space(8.0);

    // Global errors summary
    let error_kind = if overview.global_errors > 0 {
        StatusKind::Warning
    } else {
        StatusKind::Success
    };
    helpers::status_badge(
        ui,
        &format!("Global inspection errors: {}", overview.global_errors),
        error_kind,
    );

    ui.add_space(8.0);

    ui.small(
        egui::RichText::new(
            "Counts describe captured snapshot records. \
             They may include failed reads, overlapping \
             registry roots, or incomplete data.",
        )
        .color(colors.text_secondary),
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

    #[test]
    fn infer_windows_family_recognizes_windows_11_builds() {
        let (family, label) = infer_windows_family(Some(22000), Some("Windows 10 Pro"));
        assert_eq!(family, WindowsFamily::Windows11);
        assert_eq!(label, Some("Windows 11"));

        let (family, label) = infer_windows_family(Some(22621), Some("Windows 10 Pro"));
        assert_eq!(family, WindowsFamily::Windows11);
        assert_eq!(label, Some("Windows 11"));

        let (family, label) = infer_windows_family(Some(22631), Some("Windows 10 Pro"));
        assert_eq!(family, WindowsFamily::Windows11);
        assert_eq!(label, Some("Windows 11"));

        let (family, label) = infer_windows_family(Some(26100), Some("Windows 10 Pro"));
        assert_eq!(family, WindowsFamily::Windows11);
        assert_eq!(label, Some("Windows 11"));

        let (family, label) = infer_windows_family(Some(26200), Some("Windows 10 Pro"));
        assert_eq!(family, WindowsFamily::Windows11);
        assert_eq!(label, Some("Windows 11"));
    }

    #[test]
    fn infer_windows_family_recognizes_windows_10() {
        let (family, label) = infer_windows_family(Some(19045), Some("Windows 10 Pro"));
        assert_eq!(family, WindowsFamily::Windows10);
        assert_eq!(label, Some("Windows 10"));
    }

    #[test]
    fn infer_windows_family_recognizes_server() {
        let (family, label) = infer_windows_family(Some(20348), Some("Windows Server 2022"));
        assert_eq!(family, WindowsFamily::WindowsServer);
        assert_eq!(label, Some("Windows Server"));

        let (family, label) = infer_windows_family(Some(26100), Some("Windows Server 2025"));
        assert_eq!(family, WindowsFamily::WindowsServer);
        assert_eq!(label, Some("Windows Server"));
    }

    #[test]
    fn infer_windows_family_uncertain_returns_unknown() {
        // High build not in known list
        let (family, label) = infer_windows_family(Some(30000), Some("Windows 11 Enterprise"));
        assert_eq!(family, WindowsFamily::Unknown);
        assert_eq!(label, None);

        // Missing build
        let (family, label) = infer_windows_family(None, Some("Windows 10 Pro"));
        assert_eq!(family, WindowsFamily::Unknown);
        assert_eq!(label, None);
    }

    #[test]
    fn infer_windows_family_does_not_misclassify_server() {
        // Server build not in client list
        let (family, label) = infer_windows_family(Some(22621), Some("Windows Server 2022"));
        assert_eq!(family, WindowsFamily::WindowsServer);
        assert_eq!(label, Some("Windows Server"));
    }
}
