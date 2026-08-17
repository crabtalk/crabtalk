//! `impl Memory for Store`.

use crate::{
    MemoryEntry,
    interface::Memory,
    kv::{Column, KVStorage},
    store::{Store, last_segment},
    text::{TextIndex, TextSearch},
};
use anyhow::Result;

impl<K: KVStorage, T: TextSearch> Memory for Store<K, T> {
    async fn memory(&self, name: &str) -> Result<Option<MemoryEntry>> {
        self.get_json(Column::Memory, &self.memory_key(name)).await
    }

    /// Names come out of the keys themselves — an enumeration reads no
    /// entry bodies.
    async fn memory_names(&self) -> Result<Vec<String>> {
        let keys = self
            .kv
            .scan_keys(Column::Memory, &self.memory_prefix())
            .await?;
        Ok(keys.iter().filter_map(|k| last_segment(k)).collect())
    }

    async fn put_memory(&self, entry: &MemoryEntry) -> Result<()> {
        let key = self.memory_key(&entry.name);
        self.put_json(Column::Memory, &key, entry).await?;
        // Aliases are alternative search terms, so they are part of the
        // document rather than keys of their own.
        let text = match entry.aliases.is_empty() {
            true => entry.content.clone(),
            false => format!("{}\n{}", entry.aliases.join(" "), entry.content),
        };
        self.text
            .index_text(TextIndex::Memory, &key, &text, 1.0)
            .await
    }

    async fn remove_memory(&self, name: &str) -> Result<bool> {
        let key = self.memory_key(name);
        self.text.drop_text(TextIndex::Memory, &key).await?;
        self.kv.delete(Column::Memory, &key).await
    }

    async fn search_memory(&self, query: &str, limit: usize) -> Result<Vec<String>> {
        let hits = self
            .text
            .search_text(TextIndex::Memory, query, limit)
            .await?;
        Ok(hits.iter().filter_map(|h| last_segment(&h.key)).collect())
    }
}
