//! The agent's brain — named entries, searched by relevance.

use crate::{
    kv::{Column, KVStorage},
    text::{TextIndex, TextSearch},
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::future::Future;

/// One memory entry.
///
/// `kind` distinguishes what wrote it — `note` for the agent's own
/// `remember`, `archive` for a compaction summary — so a recall can tell
/// a fact the agent chose to keep from a transcript it was made to shed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub name: String,
    pub kind: String,
    pub content: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub created_at: String,
}

/// The agent's brain — named entries, searched by relevance.
pub trait Memory: KVStorage + TextSearch {
    fn memory(&self, name: &str) -> impl Future<Output = Result<Option<MemoryEntry>>> + Send {
        async move {
            self.get_json(Column::Memory, &self.key(&["memory", name]))
                .await
        }
    }

    /// Names come out of the keys themselves — an enumeration reads no
    /// entry bodies.
    fn memory_names(&self) -> impl Future<Output = Result<Vec<String>>> + Send {
        async move {
            let keys = self
                .scan_keys(Column::Memory, &self.prefix(&["memory"]))
                .await?;
            Ok(keys.iter().filter_map(|k| last_segment(k)).collect())
        }
    }

    fn put_memory(&self, entry: &MemoryEntry) -> impl Future<Output = Result<()>> + Send {
        async move {
            let key = self.key(&["memory", &entry.name]);
            self.put_json(Column::Memory, &key, entry).await?;
            // Aliases are alternative search terms, so they are part of
            // the document rather than keys of their own.
            let text = match entry.aliases.is_empty() {
                true => entry.content.clone(),
                false => format!("{}\n{}", entry.aliases.join(" "), entry.content),
            };
            self.index_text(TextIndex::Memory, &key, &text, 1.0).await
        }
    }

    fn remove_memory(&self, name: &str) -> impl Future<Output = Result<bool>> + Send {
        async move {
            let key = self.key(&["memory", name]);
            self.drop_text(TextIndex::Memory, &key).await?;
            self.delete(Column::Memory, &key).await
        }
    }

    /// Entry names ranked by relevance. The bodies are `memory` calls the
    /// caller makes for the ones it keeps.
    fn search_memory(
        &self,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<String>>> + Send {
        async move {
            let hits = self.search_text(TextIndex::Memory, query, limit).await?;
            Ok(hits.iter().filter_map(|h| last_segment(&h.key)).collect())
        }
    }
}

impl<T: KVStorage + TextSearch> Memory for T {}

/// The trailing segment of a slash-separated key.
fn last_segment(key: &[u8]) -> Option<String> {
    std::str::from_utf8(key)
        .ok()?
        .rsplit('/')
        .next()
        .map(str::to_owned)
}
