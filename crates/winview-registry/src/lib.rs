// winview-registry/src/lib.rs

pub mod reader;
pub mod version;

pub use reader::WindowsRegistryReader;
pub use version::{detect_windows_version, DetectionMethod, VersionInfo};
