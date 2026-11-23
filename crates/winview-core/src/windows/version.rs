// winview-core/src/windows/version.rs

use crate::model::DetectionMethod;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionInfo {
    pub build: Option<u32>,
    pub ubr: Option<u32>,
    pub product_name: Option<String>,
    pub detection_method: DetectionMethod,
    pub errors: Vec<String>,
}

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
