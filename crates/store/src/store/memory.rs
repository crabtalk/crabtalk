//! `impl Memory for Store`.

use crate::{
    interface::Memory,
    kv::{Column, KVStorage},
    memory::MemoryEntry,
    sql::SqlIndex,
    store::Store,
};
use anyhow::Result;

impl<K: KVStorage, Q: SqlIndex> Memory for Store<K, Q> {
    async fn memory(&self, name: &str) -> Result<Option<MemoryEntry>> {
        let key = self.tenant.key(&["memory", name]);
        self.get_json(Column::Memory, &key).await
    }

    async fn memory_names(&self) -> Result<Vec<String>> {
        self.index.memory_names().await
    }

    async fn put_memory(&self, entry: &MemoryEntry) -> Result<()> {
        let key = self.tenant.key(&["memory", &entry.name]);
        self.put_json(Column::Memory, &key, entry).await?;
        self.index.index_memory(&entry.name, &entry.content).await
    }

    async fn remove_memory(&self, name: &str) -> Result<bool> {
        let indexed = self.index.unindex_memory(name).await?;
        let key = self.tenant.key(&["memory", name]);
        let stored = self.kv.delete(Column::Memory, &key).await?;
        Ok(indexed || stored)
    }

    async fn search_memory(&self, query: &str, limit: usize) -> Result<Vec<String>> {
        self.index.search_memory(query, limit).await
    }
}
