// winview-core/src/error.rs

use thiserror::Error;

#[derive(Error, Debug)]
pub enum WinViewRsError {
    #[error("Registry error: {0}")]
    Registry(#[from] crate::registry::traits::RegistryError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerdeJson(#[from] serde_json::Error),

    #[error("Windows version detection failed: {0}")]
    VersionDetection(String),

    #[error("Unsupported operation on this Windows version: {0}")]
    UnsupportedFeature(String),

    #[error("Validation failed: {0}")]
    Validation(String),
}

// Core does NOT use anyhow — only thiserror for typed errors
