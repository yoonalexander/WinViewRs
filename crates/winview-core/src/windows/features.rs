// winview-core/src/windows/features.rs

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeatureCapabilities {
    pub compact_view: bool,
    pub classic_context_menu: bool,
    pub home_gallery_removal: bool,
    pub win10_explorer_on_11: bool,
    pub feature_18755234: bool,
    pub feature_40729001: bool,
    pub other_start_folder: bool,
    pub legacy_dialog_fix: bool,
    pub numerical_sort_policy: bool,
}

impl FeatureCapabilities {
    /// Compute capabilities from Windows build/UBR
    /// Called from app layer after version detection
    pub fn from_build_ubr(build: Option<u32>, ubr: Option<u32>) -> Self {
        let build = build.unwrap_or(0);
        let ubr = ubr.unwrap_or(0);
        Self {
            compact_view: build >= 21996,
            classic_context_menu: build >= 21996,
            home_gallery_removal: build >= 22000,
            win10_explorer_on_11: build >= 21996,
            feature_18755234: build >= 19045 && ubr >= 3754 && build < 21996,
            feature_40729001: build == 22621 && (3007..3085).contains(&ubr),
            other_start_folder: build < 26100 || (build == 26100 && ubr < 4202),
            legacy_dialog_fix: build >= 18363,
            numerical_sort_policy: true,
        }
    }
}

/// Support tier for Windows versions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupportTier {
    Supported,
    WarnUnsupported,
    RefuseWrites,
}

pub fn support_tier(build: Option<u32>, ubr: Option<u32>) -> SupportTier {
    let build = build.unwrap_or(0);
    let ubr = ubr.unwrap_or(0);
    match (build, ubr) {
        (19041..=21995, _) => SupportTier::Supported,
        (22000..=22620, _) => SupportTier::Supported,
        (22621, 3007..=3084) => SupportTier::WarnUnsupported,
        (22621, 3085..) => SupportTier::Supported,
        (22631, _) => SupportTier::Supported,
        (26100, _) => SupportTier::Supported,
        _ => SupportTier::RefuseWrites,
    }
}
