// crates/winview-core/tests/version_tests.rs

use winview_core::windows::version::VersionInfo;
use winview_core::DetectionMethod;

#[test]
fn version_info_failed_construction() {
    let info = VersionInfo::failed(vec!["error1".into(), "error2".into()]);
    assert_eq!(info.build, None);
    assert_eq!(info.ubr, None);
    assert_eq!(info.product_name, None);
    assert_eq!(info.detection_method, DetectionMethod::Failed);
    assert_eq!(info.errors.len(), 2);
}

#[test]
fn version_info_rtl_get_version_construction() {
    let info = VersionInfo::rtl_get_version(19045, Some(3754), Some("Windows 10".into()));
    assert_eq!(info.build, Some(19045));
    assert_eq!(info.ubr, Some(3754));
    assert_eq!(info.product_name, Some("Windows 10".into()));
    assert_eq!(info.detection_method, DetectionMethod::RtlGetVersion);
    assert!(info.errors.is_empty());
}

#[test]
fn version_info_registry_fallback_construction() {
    let info = VersionInfo::registry_fallback(22621, Some(3007), Some("Windows 11".into()));
    assert_eq!(info.build, Some(22621));
    assert_eq!(info.ubr, Some(3007));
    assert_eq!(info.product_name, Some("Windows 11".into()));
    assert_eq!(info.detection_method, DetectionMethod::RegistryFallback);
    assert!(info.errors.is_empty());
}

#[test]
fn version_info_serialization_roundtrip() {
    let info = VersionInfo::rtl_get_version(19045, Some(3754), Some("Windows 10".into()));
    let json = serde_json::to_string(&info).unwrap();
    let parsed: VersionInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(info, parsed);
}

#[test]
fn detection_method_serialization() {
    let methods = vec![
        DetectionMethod::Failed,
        DetectionMethod::RtlGetVersion,
        DetectionMethod::RegistryFallback,
    ];
    for method in methods {
        let json = serde_json::to_string(&method).unwrap();
        let parsed: DetectionMethod = serde_json::from_str(&json).unwrap();
        assert_eq!(method, parsed);
    }
}
