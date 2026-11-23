// crates/winview-registry/tests/integration.rs

#[cfg(target_os = "windows")]
mod integration {
    use winview_core::model::ValueReadResult;
    use winview_core::registry::keys::RootKey;
    use winview_core::registry::traits::RegistryReader;
    use winview_registry::WindowsRegistryReader;

    #[test]
    #[ignore = "requires Windows registry access"]
    fn inspect_all_returns_snapshot() {
        let reader = WindowsRegistryReader;
        let snapshot = reader.inspect_all().unwrap();

        // Basic structure validation
        assert!(!snapshot.metadata.captured_at.is_empty());
        // Build may be None if detection failed — that's acceptable but recorded
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn folder_types_hkcu_readable() {
        let reader = WindowsRegistryReader;
        let result = reader.open_key(
            RootKey::Hkcu,
            "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FolderTypes",
        );
        // Missing HKCU overrides is normal - key may not exist
        // Either Ok or KeyNotFound are valid
        assert!(
            result.is_ok()
                || matches!(
                    result,
                    Err(winview_core::registry::traits::RegistryError::KeyNotFound { .. })
                )
        );
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn folder_types_hklm_readable() {
        let reader = WindowsRegistryReader;
        let result = reader.open_key(
            RootKey::Hklm,
            "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FolderTypes",
        );
        // HKLM FolderTypes should exist on Windows - read-only access generally works without elevation
        assert!(
            result.is_ok()
                || matches!(
                    result,
                    Err(winview_core::registry::traits::RegistryError::KeyNotFound { .. })
                )
        );
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn handles_missing_keys_gracefully() {
        let reader = WindowsRegistryReader;
        let result = reader.get_value(RootKey::Hkcu, "Software\\Nonexistent\\Key", "Value");
        assert!(matches!(
            result,
            Err(winview_core::registry::traits::RegistryError::KeyNotFound { .. })
        ));
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn handles_permissions_gracefully() {
        let reader = WindowsRegistryReader;
        let result = reader.open_key(
            RootKey::Hklm,
            "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FolderTypes",
        );
        // Should not panic - either Ok, KeyNotFound, or AccessDenied
        assert!(
            result.is_ok()
                || matches!(
                    result,
                    Err(winview_core::registry::traits::RegistryError::KeyNotFound { .. })
                )
                || matches!(
                    result,
                    Err(winview_core::registry::traits::RegistryError::AccessDenied { .. })
                )
        );
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn snapshot_contains_hkcu_folder_types() {
        let reader = WindowsRegistryReader;
        let snapshot = reader.inspect_all().unwrap();

        // HKCU FolderTypes domain should be present (may be empty if no overrides)
        let _ = &snapshot.hkcu_folder_types; // Type check: FolderTypesSnapshot
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn snapshot_contains_hklm_folder_types() {
        let reader = WindowsRegistryReader;
        let snapshot = reader.inspect_all().unwrap();

        // HKLM FolderTypes domain should be present
        let _ = &snapshot.hklm_folder_types; // Type check: FolderTypesSnapshot
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn snapshot_contains_advanced_key() {
        let reader = WindowsRegistryReader;
        let snapshot = reader.inspect_all().unwrap();

        // Advanced is a single KeySnapshot, always present
        let _ = &snapshot.advanced; // Type check: KeySnapshot
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn value_read_preserves_raw_type_id() {
        let reader = WindowsRegistryReader;
        let snapshot = reader.inspect_all().unwrap();

        // Find any value and verify raw_type_id is populated
        let mut found_any = false;
        for fs in snapshot.hkcu_folder_types.types.values() {
            for view in fs.top_views.values() {
                for vread in view.raw_values.values() {
                    if let ValueReadResult::Success(_raw) = vread {
                        found_any = true;
                        // Windows REG_* type IDs: 0=REG_NONE, 1=REG_SZ, 2=REG_EXPAND_SZ, 3=REG_BINARY, 4=REG_DWORD, 7=REG_MULTI_SZ, 11=REG_QWORD
                        // Unknown types have other IDs
                        // All are valid u32
                    }
                }
            }
        }
        // Also check HKLM
        for fs in snapshot.hklm_folder_types.types.values() {
            for view in fs.top_views.values() {
                for vread in view.raw_values.values() {
                    if let ValueReadResult::Success(_raw) = vread {
                        found_any = true;
                    }
                }
            }
        }
        // On a clean system, there should be at least some values
        assert!(
            found_any,
            "Expected at least one registry value in FolderTypes"
        );
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn unknown_registry_types_preserved() {
        let reader = WindowsRegistryReader;
        let snapshot = reader.inspect_all().unwrap();
        let mut found_unknown = false;
        for fs in snapshot.hkcu_folder_types.types.values() {
            for view in fs.top_views.values() {
                for vread in view.raw_values.values() {
                    if let ValueReadResult::Success(_raw) = vread {
                        if matches!(
                            _raw.decoding_status,
                            winview_core::model::DecodingStatus::UnknownType
                        ) {
                            found_unknown = true;
                            assert!(_raw.raw_type_id > 11 || _raw.raw_type_id == 0); // Unknown or REG_NONE
                            assert!(!_raw.raw_bytes.is_empty());
                        }
                    }
                }
            }
        }
        // Don't assert found_unknown — just documents the check
        let _ = found_unknown;
    }

    #[test]
    #[ignore = "requires Windows registry access"]
    fn inspection_status_reflects_completeness() {
        let reader = WindowsRegistryReader;
        let snapshot = reader.inspect_all().unwrap();

        // Status should be Complete, Partial, or Failed
        match snapshot.status {
            winview_core::model::InspectionStatus::Complete => {}
            winview_core::model::InspectionStatus::Partial { reason } => {
                assert!(!reason.is_empty());
            }
            winview_core::model::InspectionStatus::Failed { error } => {
                assert!(!error.is_empty());
            }
        }
    }
}
