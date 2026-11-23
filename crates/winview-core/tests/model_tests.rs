// crates/winview-core/tests/model_tests.rs

use winview_core::model::{
    DecodingStatus, InspectionSnapshot, ObservationSource, RawRegistryValue, ReadError,
    RegistryValue, ValueReadResult,
};
use winview_core::registry::keys::{RootKey, ValueTypeName};
use winview_core::registry::traits::RegistryReader;
use winview_core::test_support::FakeRegistryReader;

#[test]
fn inspection_snapshot_serialization() {
    let snapshot = InspectionSnapshot::default();
    let json = serde_json::to_string(&snapshot).unwrap();
    let parsed: InspectionSnapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(snapshot.metadata.captured_at, parsed.metadata.captured_at);
}

#[test]
fn registry_value_type_preservation() {
    let values = vec![
        RegistryValue::String("test".into()),
        RegistryValue::Dword(42),
        RegistryValue::Qword(1234567890123),
        RegistryValue::Binary(vec![0x01, 0x02, 0x03]),
        RegistryValue::MultiString(vec!["a".into(), "b".into()]),
        // REG_NONE and unknown types are NOT in RegistryValue enum
    ];
    for v in values {
        let json = serde_json::to_string(&v).unwrap();
        let parsed: RegistryValue = serde_json::from_str(&json).unwrap();
        assert_eq!(v, parsed);
    }
}

#[test]
fn raw_registry_value_always_has_raw_bytes() {
    let read = RawRegistryValue {
        key_path: r"Software\Test".to_string(),
        value_name: "Value".to_string(),
        root: RootKey::Hkcu,
        raw_type_id: 4,
        raw_bytes: vec![0x7B, 0x00, 0x00, 0x00], // 123 in little-endian
        decoded: Some(RegistryValue::Dword(123)),
        decoding_status: DecodingStatus::Decoded,
        source: ObservationSource::HkcuAdvanced,
    };
    let json = serde_json::to_string(&read).unwrap();
    let parsed: RawRegistryValue = serde_json::from_str(&json).unwrap();
    assert_eq!(read.raw_bytes, parsed.raw_bytes);
    assert_eq!(read.raw_type_id, parsed.raw_type_id);
    assert!(matches!(parsed.decoding_status, DecodingStatus::Decoded));
}

#[test]
fn raw_registry_value_malformed_preserves_bytes() {
    let read = RawRegistryValue {
        key_path: r"Software\Test".to_string(),
        value_name: "BadDword".to_string(),
        root: RootKey::Hkcu,
        raw_type_id: 4,
        raw_bytes: vec![0x7B, 0x00], // Wrong length for DWORD
        decoded: None,
        decoding_status: DecodingStatus::Malformed {
            reason: "DWORD length != 4".into(),
        },
        source: ObservationSource::HkcuAdvanced,
    };
    let json = serde_json::to_string(&read).unwrap();
    let parsed: RawRegistryValue = serde_json::from_str(&json).unwrap();
    assert_eq!(read.raw_bytes, parsed.raw_bytes);
    assert!(matches!(
        parsed.decoding_status,
        DecodingStatus::Malformed { .. }
    ));
    assert!(parsed.decoded.is_none());
}

#[test]
fn raw_registry_value_unknown_type_preserves_bytes_and_id() {
    let read = RawRegistryValue {
        key_path: r"Software\Test".to_string(),
        value_name: "Unknown".to_string(),
        root: RootKey::Hkcu,
        raw_type_id: 999, // Unknown type
        raw_bytes: vec![0xFF, 0xEE, 0xDD],
        decoded: None,
        decoding_status: DecodingStatus::UnknownType,
        source: ObservationSource::HkcuAdvanced,
    };
    let json = serde_json::to_string(&read).unwrap();
    let parsed: RawRegistryValue = serde_json::from_str(&json).unwrap();
    assert_eq!(read.raw_bytes, parsed.raw_bytes);
    assert_eq!(read.raw_type_id, parsed.raw_type_id);
    assert!(matches!(
        parsed.decoding_status,
        DecodingStatus::UnknownType
    ));
}

#[test]
fn value_read_result_success_serialization() {
    let read = RawRegistryValue {
        key_path: r"Software\Test".to_string(),
        value_name: "Value".to_string(),
        root: RootKey::Hkcu,
        raw_type_id: 4,
        raw_bytes: vec![0x7B, 0x00, 0x00, 0x00],
        decoded: Some(RegistryValue::Dword(123)),
        decoding_status: DecodingStatus::Decoded,
        source: ObservationSource::HkcuAdvanced,
    };
    let result: ValueReadResult = ValueReadResult::Success(read);
    let json = serde_json::to_string(&result).unwrap();
    let parsed: ValueReadResult = serde_json::from_str(&json).unwrap();
    assert!(
        matches!(parsed, ValueReadResult::Success(r) if r.decoded == Some(RegistryValue::Dword(123)))
    );
}

#[test]
fn value_read_result_failure_serialization() {
    let error = ReadError::KeyNotFound {
        path: r"Software\Missing".to_string(),
    };
    let result: ValueReadResult = ValueReadResult::Failure(error);
    let json = serde_json::to_string(&result).unwrap();
    let parsed: ValueReadResult = serde_json::from_str(&json).unwrap();
    assert!(matches!(
        parsed,
        ValueReadResult::Failure(ReadError::KeyNotFound { .. })
    ));
}

#[test]
fn inspection_error_captures_context() {
    let err = winview_core::model::InspectionError {
        key_path: r"Software\Missing".to_string(),
        root: RootKey::Hkcu,
        operation: "open_key".to_string(),
        error: "Key not found".to_string(),
    };
    let json = serde_json::to_string(&err).unwrap();
    let parsed: winview_core::model::InspectionError = serde_json::from_str(&json).unwrap();
    assert_eq!(err.key_path, parsed.key_path);
    assert_eq!(err.root, parsed.root);
}

#[test]
fn fake_registry_reader_basic_operations() {
    let fake = FakeRegistryReader::new();
    fake.insert_value(
        RootKey::Hkcu,
        r"Software\Test",
        "Value",
        4,
        vec![0x7B, 0x00, 0x00, 0x00],
        Some(RegistryValue::Dword(123)),
    );

    let result = fake.get_value(RootKey::Hkcu, r"Software\Test", "Value");
    assert!(result.is_ok());
    let (type_name, value) = result.unwrap();
    assert_eq!(type_name, ValueTypeName::RegDword);
    assert_eq!(value, RegistryValue::Dword(123));
}

#[test]
fn fake_registry_reader_missing_key_returns_error() {
    let fake = FakeRegistryReader::new();
    let result = fake.get_value(RootKey::Hkcu, r"Software\Missing", "Value");
    assert!(matches!(
        result,
        Err(winview_core::registry::traits::RegistryError::KeyNotFound { .. })
    ));
}

#[test]
fn fake_registry_reader_missing_value_returns_error() {
    let fake = FakeRegistryReader::new();
    fake.insert_key(
        RootKey::Hkcu,
        r"Software\Test",
        winview_core::test_support::FakeRegistryKey::new(),
    );
    let result = fake.get_value(RootKey::Hkcu, r"Software\Test", "MissingValue");
    assert!(matches!(
        result,
        Err(winview_core::registry::traits::RegistryError::ValueNotFound { .. })
    ));
}
