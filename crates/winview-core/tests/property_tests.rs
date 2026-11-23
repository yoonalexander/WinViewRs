// crates/winview-core/tests/property_tests.rs

use proptest::prelude::*;
use winview_core::model::{DecodingStatus, ObservationSource, RawRegistryValue, RegistryValue};
use winview_core::RootKey;

proptest! {
    #[test]
    fn registry_value_roundtrip(v in arb_registry_value()) {
        let json = serde_json::to_string(&v).unwrap();
        let parsed: RegistryValue = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(v, parsed);
    }

    #[test]
    fn raw_registry_value_roundtrip(
        path in ".*",
        name in ".*",
        root in prop::sample::select(vec![RootKey::Hkcu, RootKey::Hklm]),
        type_id in 0u32..200,
        bytes in proptest::collection::vec(any::<u8>(), 0..100),
        source in prop::sample::select(vec![
            ObservationSource::HkcuAdvanced,
            ObservationSource::HkcuFolderTypes,
            ObservationSource::HklmFolderTypes,
        ]),
    ) {
        let read = RawRegistryValue {
            key_path: path,
            value_name: name,
            root,
            raw_type_id: type_id,
            raw_bytes: bytes.clone(),
            decoded: None,
            decoding_status: DecodingStatus::UnknownType,
            source,
        };
        let json = serde_json::to_string(&read).unwrap();
        let parsed: RawRegistryValue = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(read.raw_bytes, parsed.raw_bytes);
        prop_assert_eq!(read.raw_type_id, parsed.raw_type_id);
    }

    #[test]
    fn snapshot_metadata_roundtrip(
        build in proptest::option::of(10000u32..30000),
        ubr in proptest::option::of(0u32..10000),
        product in proptest::option::of(".*"),
        method in prop::sample::select(vec![
            winview_core::DetectionMethod::Failed,
            winview_core::DetectionMethod::RtlGetVersion,
            winview_core::DetectionMethod::RegistryFallback,
        ]),
    ) {
        let meta = winview_core::model::SnapshotMetadata {
            captured_at: "2024-01-01T00:00:00Z".to_string(),
            windows_build: build,
            windows_ubr: ubr,
            windows_product_name: product,
            detection_method: method,
        };
        let json = serde_json::to_string(&meta).unwrap();
        let parsed: winview_core::model::SnapshotMetadata = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(meta.windows_build, parsed.windows_build);
        prop_assert_eq!(meta.windows_ubr, parsed.windows_ubr);
    }
}

fn arb_registry_value() -> impl Strategy<Value = RegistryValue> {
    prop_oneof![
        ".*".prop_map(RegistryValue::String),
        any::<u32>().prop_map(RegistryValue::Dword),
        any::<u64>().prop_map(RegistryValue::Qword),
        proptest::collection::vec(any::<u8>(), 0..100).prop_map(RegistryValue::Binary),
        proptest::collection::vec(".*", 0..10).prop_map(RegistryValue::MultiString),
        // No RegistryValue::None — unknown types use RawRegistryValue directly
    ]
}
