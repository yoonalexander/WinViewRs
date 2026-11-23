// winview-core/src/registry/traits.rs

use crate::model::{InspectionSnapshot, RegistryValue};
use crate::registry::keys::{RootKey, ValueTypeName};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RegistryError {
    #[error("Key not found: {root:?}\\{path}")]
    KeyNotFound { root: RootKey, path: String },

    #[error("Value not found: {root:?}\\{path}\\{name}")]
    ValueNotFound {
        root: RootKey,
        path: String,
        name: String,
    },

    #[error("Type mismatch: expected {expected:?}, got {actual:?}")]
    TypeMismatch {
        expected: ValueTypeName,
        actual: ValueTypeName,
    },

    #[error("Access denied: {root:?}\\{path}")]
    AccessDenied { root: RootKey, path: String },

    #[error("Windows API error: {0}")]
    WindowsApi(String),

    #[error("Invalid data: {0}")]
    InvalidData(String),
}

pub type RegistryResult<T> = Result<T, RegistryError>;

/// Read-only registry access trait — implemented by platform adapter
/// NOTE: Key is Send but NOT Sync (matches windows_registry::Key)
/// Value is !Send + !Sync
pub trait RegistryReader: Send + Sync {
    /// Open a key for reading
    fn open_key(&self, root: RootKey, path: &str) -> RegistryResult<Box<dyn RegistryKey>>;

    /// Enumerate subkey names
    fn enum_keys(&self, root: RootKey, path: &str) -> RegistryResult<Vec<String>>;

    /// Get a value with its type
    fn get_value(
        &self,
        root: RootKey,
        path: &str,
        name: &str,
    ) -> RegistryResult<(ValueTypeName, RegistryValue)>;

    /// Get value type only
    fn get_value_type(
        &self,
        root: RootKey,
        path: &str,
        name: &str,
    ) -> RegistryResult<ValueTypeName>;

    /// Capture a complete inspection snapshot (Phase 0/1 primary operation)
    fn inspect_all(&self) -> RegistryResult<InspectionSnapshot>;
}

/// Read-only registry key handle
/// NOTE: Key is Send but NOT Sync (mirrors windows_registry::Key)
pub trait RegistryKey: Send {
    fn enum_values(&self) -> RegistryResult<Vec<(String, ValueTypeName)>>;
    fn enum_subkeys(&self) -> RegistryResult<Vec<String>>;
    fn get_value(&self, name: &str) -> RegistryResult<(ValueTypeName, RegistryValue)>;
    fn get_value_type(&self, name: &str) -> RegistryResult<ValueTypeName>;
    fn open_subkey(&self, name: &str) -> RegistryResult<Box<dyn RegistryKey>>;
}
