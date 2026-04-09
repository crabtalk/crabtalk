//! In-memory [`Storage`] implementation for tests and tooling.

use crate::storage::Storage;
use anyhow::Result;
use std::{collections::BTreeMap, sync::Mutex};

/// `BTreeMap`-backed [`Storage`] implementation. `BTreeMap` so `list` is
/// naturally sorted; `Mutex` so the type is `Sync` despite interior
/// mutability.
#[derive(Default)]
pub struct MemStorage {
    inner: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl MemStorage {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Storage for MemStorage {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .inner
            .lock()
            .expect("MemStorage mutex poisoned")
            .get(key)
            .cloned())
    }

    fn put(&self, key: &str, value: &[u8]) -> Result<()> {
        self.inner
            .lock()
            .expect("MemStorage mutex poisoned")
            .insert(key.to_owned(), value.to_vec());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.inner
            .lock()
            .expect("MemStorage mutex poisoned")
            .remove(key);
        Ok(())
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let guard = self.inner.lock().expect("MemStorage mutex poisoned");
        Ok(guard
            .range(prefix.to_owned()..)
            .take_while(|(k, _)| k.starts_with(prefix))
            .map(|(k, _)| k.clone())
            .collect())
    }
}
