// winview-core/src/inspection.rs

use crate::model::{FolderTypesSnapshot, InspectionSnapshot, KeySnapshot, ValueReadResult};

/// Counts recorded inspection errors in a key
/// and all its descendants.
pub fn count_key_read_errors(key: &KeySnapshot) -> usize {
    key.read_errors.len()
        + key
            .subkeys
            .values()
            .map(count_key_read_errors)
            .sum::<usize>()
}

/// Counts hive-level and per-folder errors.
pub fn count_folder_types_read_errors(folder_types: &FolderTypesSnapshot) -> usize {
    folder_types.read_errors.len()
        + folder_types
            .types
            .values()
            .map(|folder| folder.read_errors.len())
            .sum::<usize>()
}

/// Counts recorded inspection errors across
/// every domain without double-counting failed
/// value observations.
pub fn count_snapshot_read_errors(snapshot: &InspectionSnapshot) -> usize {
    snapshot.errors.len()
        + count_folder_types_read_errors(&snapshot.hkcu_folder_types)
        + count_folder_types_read_errors(&snapshot.hklm_folder_types)
        + count_key_read_errors(&snapshot.advanced)
        + [
            &snapshot.bags,
            &snapshot.streams,
            &snapshot.clsid,
            &snapshot.policies,
        ]
        .iter()
        .map(|domain| domain.values().map(count_key_read_errors).sum::<usize>())
        .sum::<usize>()
}

fn key_has_successful_observations(key: &KeySnapshot) -> bool {
    key.values
        .values()
        .any(|value| matches!(value, ValueReadResult::Success(_)))
        || key.subkeys.values().any(key_has_successful_observations)
}

fn folder_types_has_successful_observations(folder_types: &FolderTypesSnapshot) -> bool {
    folder_types.types.values().any(|folder| {
        folder.canonical_name.is_some()
            || folder.top_views.values().any(|view| {
                view.raw_values
                    .values()
                    .any(|value| matches!(value, ValueReadResult::Success(_)))
            })
    })
}

/// Returns true when at least one registry value
/// or CanonicalName was successfully captured.
///
/// Raw observations count even when their values
/// could not be decoded. Empty containers and
/// failed reads do not count.
pub fn snapshot_has_successful_observations(snapshot: &InspectionSnapshot) -> bool {
    folder_types_has_successful_observations(&snapshot.hkcu_folder_types)
        || folder_types_has_successful_observations(&snapshot.hklm_folder_types)
        || key_has_successful_observations(&snapshot.advanced)
        || [
            &snapshot.bags,
            &snapshot.streams,
            &snapshot.clsid,
            &snapshot.policies,
        ]
        .iter()
        .any(|domain| domain.values().any(key_has_successful_observations))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        DecodingStatus, FolderTypeSnapshot, InspectionError, ObservationSource, RawRegistryValue,
        ReadError, RegistryValue, ViewSnapshot,
    };
    use crate::registry::keys::RootKey;

    fn error(path: &str) -> InspectionError {
        InspectionError {
            key_path: path.into(),
            root: RootKey::Hkcu,
            operation: "get_value".into(),
            error: "injected failure".into(),
        }
    }

    fn successful_value() -> ValueReadResult {
        ValueReadResult::Success(RawRegistryValue {
            key_path: "Software\\Test".into(),
            value_name: "Mode".into(),
            root: RootKey::Hkcu,
            raw_type_id: 4,
            raw_bytes: 42u32.to_le_bytes().to_vec(),
            decoded: Some(RegistryValue::Dword(42)),
            decoding_status: DecodingStatus::Decoded,
            source: ObservationSource::HkcuAdvanced,
        })
    }

    fn key_with_nested_errors() -> KeySnapshot {
        let mut key = KeySnapshot::default();

        key.read_errors.push(error("root"));

        let mut child = KeySnapshot::default();

        child.read_errors.push(error("root\\child"));

        key.subkeys.insert("root\\child".into(), child);

        key
    }

    #[test]
    fn shared_error_count_covers_all_domains() {
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

            assert_eq!(count_folder_types_read_errors(folder_types), 2);
        }

        snapshot.advanced = key_with_nested_errors();

        assert_eq!(count_key_read_errors(&snapshot.advanced), 2);

        for domain in [
            &mut snapshot.bags,
            &mut snapshot.streams,
            &mut snapshot.clsid,
            &mut snapshot.policies,
        ] {
            domain.insert("root".into(), key_with_nested_errors());
        }

        assert_eq!(count_snapshot_read_errors(&snapshot), 15);
    }

    #[test]
    fn shared_error_count_ignores_failed_value_records() {
        let mut snapshot = InspectionSnapshot::default();

        snapshot.advanced.values.insert(
            "Missing".into(),
            ValueReadResult::Failure(ReadError::ValueNotFound {
                name: "Missing".into(),
            }),
        );

        assert_eq!(count_snapshot_read_errors(&snapshot), 0);

        snapshot.advanced.values.insert(
            "Denied".into(),
            ValueReadResult::Failure(ReadError::AccessDenied {
                path: "Advanced".into(),
            }),
        );

        snapshot.advanced.read_errors.push(error("Advanced"));

        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert!(!snapshot_has_successful_observations(&snapshot));

        snapshot
            .advanced
            .values
            .insert("Mode".into(), successful_value());

        assert_eq!(count_snapshot_read_errors(&snapshot), 1);

        assert!(snapshot_has_successful_observations(&snapshot));
    }

    #[test]
    fn shared_observation_detection_covers_all_domains() {
        for index in 0..7 {
            let mut snapshot = InspectionSnapshot::default();

            assert!(!snapshot_has_successful_observations(&snapshot));

            match index {
                0 => {
                    snapshot
                        .advanced
                        .values
                        .insert("Mode".into(), successful_value());
                }

                1..=4 => {
                    let domain = match index {
                        1 => &mut snapshot.bags,
                        2 => &mut snapshot.streams,
                        3 => &mut snapshot.clsid,
                        4 => &mut snapshot.policies,
                        _ => unreachable!(),
                    };

                    let mut root = KeySnapshot::default();

                    let mut child = KeySnapshot::default();

                    child.values.insert("Mode".into(), successful_value());

                    root.subkeys.insert("child".into(), child);

                    domain.insert("root".into(), root);
                }

                5 | 6 => {
                    let target = if index == 5 {
                        &mut snapshot.hkcu_folder_types
                    } else {
                        &mut snapshot.hklm_folder_types
                    };

                    let mut folder = FolderTypeSnapshot::default();

                    let mut view = ViewSnapshot::default();

                    view.raw_values.insert("Mode".into(), successful_value());

                    folder.top_views.insert("view".into(), view);

                    target.types.insert("folder".into(), folder);
                }

                _ => unreachable!(),
            }

            assert!(
                snapshot_has_successful_observations(&snapshot),
                "Observation not detected in domain {index}"
            );

            assert_eq!(count_snapshot_read_errors(&snapshot), 0);
        }
    }

    #[test]
    fn shared_observation_detection_preserves_raw_and_canonical_semantics() {
        for decoding_status in [
            DecodingStatus::UnknownType,
            DecodingStatus::Malformed {
                reason: "injected malformed value".into(),
            },
        ] {
            let mut snapshot = InspectionSnapshot::default();

            snapshot.advanced.values.insert(
                "Raw".into(),
                ValueReadResult::Success(RawRegistryValue {
                    key_path: "Advanced".into(),
                    value_name: "Raw".into(),
                    root: RootKey::Hkcu,
                    raw_type_id: 999,
                    raw_bytes: Vec::new(),
                    decoded: None,
                    decoding_status,
                    source: ObservationSource::HkcuAdvanced,
                }),
            );

            assert!(snapshot_has_successful_observations(&snapshot));
        }

        for root in [RootKey::Hkcu, RootKey::Hklm] {
            let mut snapshot = InspectionSnapshot::default();

            let target = match root {
                RootKey::Hkcu => &mut snapshot.hkcu_folder_types,
                RootKey::Hklm => &mut snapshot.hklm_folder_types,
            };

            target.types.insert(
                "folder".into(),
                FolderTypeSnapshot {
                    guid: "folder".into(),
                    ..Default::default()
                },
            );

            assert!(!snapshot_has_successful_observations(&snapshot));

            let target = match root {
                RootKey::Hkcu => &mut snapshot.hkcu_folder_types,
                RootKey::Hklm => &mut snapshot.hklm_folder_types,
            };

            target.types.get_mut("folder").unwrap().canonical_name = Some(String::new());

            assert!(snapshot_has_successful_observations(&snapshot));
        }
    }
}
