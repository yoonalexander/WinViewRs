// winview-core/src/lib.rs

pub mod error;
pub mod model;
pub mod registry;
pub mod test_support;
pub mod windows;

pub use error::WinViewRsError;
pub use model::{
    DecodingStatus, DetectionMethod, FolderTypesSnapshot, InspectionError, InspectionSnapshot,
    InspectionStatus, KeySnapshot, ObservationSource, RawRegistryValue, ReadError, RegistryValue,
    SnapshotMetadata, ValueReadResult, ViewSnapshot,
};
pub use registry::keys::{RootKey, ValueTypeName};
pub use registry::traits::{RegistryError, RegistryKey, RegistryReader, RegistryResult};
pub use windows::features::{support_tier, FeatureCapabilities, SupportTier};
pub use windows::version::VersionInfo;
