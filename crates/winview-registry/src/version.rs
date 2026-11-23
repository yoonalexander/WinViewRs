// winview-registry/src/version.rs

use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::Foundation::NTSTATUS;
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows_registry::LOCAL_MACHINE;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionInfo {
    pub build: Option<u32>,
    pub ubr: Option<u32>,
    pub product_name: Option<String>,
    pub detection_method: DetectionMethod,
    pub errors: Vec<String>,
}

pub use winview_core::model::DetectionMethod;

impl VersionInfo {
    pub fn failed(errors: Vec<String>) -> Self {
        Self {
            build: None,
            ubr: None,
            product_name: None,
            detection_method: DetectionMethod::Failed,
            errors,
        }
    }

    pub fn rtl_get_version(build: u32, ubr: Option<u32>, product_name: Option<String>) -> Self {
        Self {
            build: Some(build),
            ubr,
            product_name,
            detection_method: DetectionMethod::RtlGetVersion,
            errors: Vec::new(),
        }
    }

    pub fn registry_fallback(build: u32, ubr: Option<u32>, product_name: Option<String>) -> Self {
        Self {
            build: Some(build),
            ubr,
            product_name,
            detection_method: DetectionMethod::RegistryFallback,
            errors: Vec::new(),
        }
    }
}

pub fn detect_windows_version() -> VersionInfo {
    // Primary: RtlGetVersion from ntdll (WDK API)
    if let Some(info) = get_version_rtl() {
        return info;
    }

    // Fallback: registry cross-check
    if let Some(info) = get_version_registry() {
        return info;
    }

    VersionInfo::failed(vec![
        "Both RtlGetVersion and registry fallback failed".into()
    ])
}

fn get_version_rtl() -> Option<VersionInfo> {
    // RtlGetVersion signature: NTSTATUS RtlGetVersion(OSVERSIONINFOW*)
    // OSVERSIONINFOW contains: dwOSVersionInfoSize, dwMajorVersion, dwMinorVersion,
    // dwBuildNumber, dwPlatformId, szCSDVersion
    // UBR is NOT in this struct — must read from registry

    // SAFETY: RtlGetVersion is a documented, stable API
    let mut info: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
    info.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;

    // Link ntdll and call RtlGetVersion via windows crate
    // Requires: windows = { features = ["Wdk_System_SystemServices", "Win32_Foundation"] }
    let status = unsafe { RtlGetVersion(&mut info) };

    if status != NTSTATUS(0) {
        return None;
    }

    let build = info.dwBuildNumber;

    // Read UBR from registry
    let ubr = read_ubr_from_registry();
    let product_name = read_product_name_from_registry();

    Some(VersionInfo::rtl_get_version(build, ubr, product_name))
}

fn read_ubr_from_registry() -> Option<u32> {
    let key = LOCAL_MACHINE
        .open(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .ok()?;
    key.get_u32("UBR").ok()
}

fn read_product_name_from_registry() -> Option<String> {
    let key = LOCAL_MACHINE
        .open(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .ok()?;
    key.get_string("ProductName").ok()
}

fn get_version_registry() -> Option<VersionInfo> {
    // Read from HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion
    // CurrentBuild, CurrentBuildNumber (string), UBR, ProductName
    let key = LOCAL_MACHINE
        .open(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .ok()?;

    // CurrentBuild is stored as string in registry
    let build_str: String = key.get_string("CurrentBuild").ok()?;
    let build: u32 = build_str.parse().ok()?;
    let ubr: Option<u32> = key.get_u32("UBR").ok();
    let product_name: Option<String> = key.get_string("ProductName").ok();

    Some(VersionInfo::registry_fallback(build, ubr, product_name))
}
