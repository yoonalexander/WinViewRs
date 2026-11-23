// winview-core/src/test_support.rs

use crate::model::DecodingStatus;
use crate::model::InspectionSnapshot;
use crate::model::RegistryValue;
use crate::registry::keys::{RootKey, ValueTypeName};
use crate::registry::traits::{RegistryError, RegistryKey, RegistryReader, RegistryResult};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// In-memory fake registry for deterministic testing
/// Implements RegistryReader/RegistryKey traits using HashMap storage
impl Default for FakeRegistryReader {
    fn default() -> Self {
        Self::new()
    }
}

pub struct FakeRegistryReader {
    // (root, path) -> FakeRegistryKey
    keys: Arc<Mutex<BTreeMap<(RootKey, String), FakeRegistryKey>>>,
}

impl FakeRegistryReader {
    pub fn new() -> Self {
        Self {
            keys: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn insert_key(&self, root: RootKey, path: &str, key: FakeRegistryKey) {
        self.keys
            .lock()
            .unwrap()
            .insert((root, path.to_string()), key);
    }

    pub fn insert_value(
        &self,
        root: RootKey,
        path: &str,
        name: &str,
        type_id: u32,
        bytes: Vec<u8>,
        decoded: Option<RegistryValue>,
    ) {
        let mut keys = self.keys.lock().unwrap();
        let entry = keys.entry((root, path.to_string())).or_default();
        let status = if decoded.is_some() {
            DecodingStatus::Decoded
        } else {
            DecodingStatus::UnknownType
        };
        entry
            .values
            .insert(name.to_string(), (type_id, bytes, decoded, status));
    }
}

#[derive(Clone)]
pub struct FakeRegistryKey {
    values: BTreeMap<String, (u32, Vec<u8>, Option<RegistryValue>, DecodingStatus)>, // name -> (type_id, raw_bytes, decoded, status)
    subkeys: BTreeMap<String, FakeRegistryKey>,
}

impl Default for FakeRegistryKey {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeRegistryKey {
    pub fn new() -> Self {
        Self {
            values: BTreeMap::new(),
            subkeys: BTreeMap::new(),
        }
    }
}

impl RegistryReader for FakeRegistryReader {
    fn open_key(&self, root: RootKey, path: &str) -> RegistryResult<Box<dyn RegistryKey>> {
        let keys = self.keys.lock().unwrap();
        keys.get(&(root, path.to_string()))
            .map(|k| Box::new(FakeRegistryKeyHandle { inner: k.clone() }) as Box<dyn RegistryKey>)
            .ok_or(RegistryError::KeyNotFound {
                root,
                path: path.to_string(),
            })
    }

    fn enum_keys(&self, root: RootKey, path: &str) -> RegistryResult<Vec<String>> {
        let key = self.open_key(root, path)?;
        key.enum_subkeys()
    }

    fn get_value(
        &self,
        root: RootKey,
        path: &str,
        name: &str,
    ) -> RegistryResult<(ValueTypeName, Vec<u8>, RegistryValue, DecodingStatus)> {
        let key = self.open_key(root, path)?;
        key.get_value(name)
    }

    fn get_value_type(
        &self,
        root: RootKey,
        path: &str,
        name: &str,
    ) -> RegistryResult<ValueTypeName> {
        let key = self.open_key(root, path)?;
        key.get_value_type(name)
    }

    fn inspect_all(&self) -> RegistryResult<InspectionSnapshot> {
        // Return minimal valid snapshot for testing
        Ok(InspectionSnapshot::default())
    }
}

struct FakeRegistryKeyHandle {
    inner: FakeRegistryKey,
}

impl RegistryKey for FakeRegistryKeyHandle {
    fn enum_values(&self) -> RegistryResult<Vec<(String, ValueTypeName)>> {
        Ok(self
            .inner
            .values
            .iter()
            .map(|(name, (type_id, _, _, _))| (name.clone(), type_id_to_name(*type_id)))
            .collect())
    }

    fn enum_subkeys(&self) -> RegistryResult<Vec<String>> {
        Ok(self.inner.subkeys.keys().cloned().collect())
    }

    fn get_value(&self, name: &str) -> RegistryResult<(ValueTypeName, Vec<u8>, RegistryValue, DecodingStatus)> {
        let (type_id, raw_bytes, decoded, status) =
            self.inner
                .values
                .get(name)
                .ok_or(RegistryError::ValueNotFound {
                    root: RootKey::Hkcu,
                    path: "".into(),
                    name: name.into(),
                })?;
        let decoded = decoded.clone().ok_or(RegistryError::InvalidData(
            "no decoded value in fake".into(),
        ))?;
        Ok((type_id_to_name(*type_id), raw_bytes.clone(), decoded, status.clone()))
    }

    fn get_value_type(&self, name: &str) -> RegistryResult<ValueTypeName> {
        let (type_id, _, _, _) =
            self.inner
                .values
                .get(name)
                .ok_or(RegistryError::ValueNotFound {
                    root: RootKey::Hkcu,
                    path: "".into(),
                    name: name.into(),
                })?;
        Ok(type_id_to_name(*type_id))
    }

    fn open_subkey(&self, name: &str) -> RegistryResult<Box<dyn RegistryKey>> {
        self.inner
            .subkeys
            .get(name)
            .map(|k| Box::new(FakeRegistryKeyHandle { inner: k.clone() }) as Box<dyn RegistryKey>)
            .ok_or(RegistryError::KeyNotFound {
                root: RootKey::Hkcu,
                path: name.into(),
            })
    }
}

fn type_id_to_name(id: u32) -> ValueTypeName {
    match id {
        1 => ValueTypeName::RegSz,
        2 => ValueTypeName::RegExpandSz,
        3 => ValueTypeName::RegBinary,
        4 => ValueTypeName::RegDword,
        7 => ValueTypeName::RegMultiSz,
        11 => ValueTypeName::RegQword,
        _ => ValueTypeName::Other(id),
    }
}
