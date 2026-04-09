//! Built-in memory — file-per-entry persistence behind the runtime
//! [`Storage`](crate::storage::Storage) trait.
//!
//! Key layout (rooted at the Storage backend's root — in production this
//! is the daemon config dir):
//!
//! - `memory/entries/<slug>.md` — one file per remembered entry.
//! - `memory/MEMORY.md` — curated overview injected into every agent.
//!
//! Legacy pre-trait installs used `memory/memory.md`, `memory/user.md`,
//! `memory/facts.toml`; [`Memory::open`] migrates those on first load.

use crate::{config::MemoryConfig, storage::Storage};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use wcore::model::{HistoryEntry, Role};

pub mod bm25;
pub mod entry;
pub mod tool;

use entry::MemoryEntry;

const MEMORY_PROMPT: &str = include_str!("../../prompts/memory.md");

pub const DEFAULT_SOUL: &str = include_str!("../../prompts/crab.md");

/// Storage key prefix for individual memory entries.
pub(crate) const ENTRIES_PREFIX: &str = "memory/entries/";
/// Storage key for the curated MEMORY.md overview.
pub(crate) const INDEX_KEY: &str = "memory/MEMORY.md";

// Legacy keys (pre-Storage-trait). Still honored by migrate_legacy for
// one-shot conversion into the new layout.
const LEGACY_MEMORY_KEY: &str = "memory/memory.md";
const LEGACY_USER_KEY: &str = "memory/user.md";
const LEGACY_FACTS_KEY: &str = "memory/facts.toml";
const LEGACY_MEMORY_BAK: &str = "memory/memory.md.bak";
const LEGACY_USER_BAK: &str = "memory/user.md.bak";
const LEGACY_FACTS_BAK: &str = "memory/facts.toml.bak";

pub struct Memory {
    storage: Arc<dyn Storage>,
    entries: RwLock<HashMap<String, MemoryEntry>>,
    index: RwLock<String>,
    config: MemoryConfig,
}

impl Memory {
    /// Open (or create) memory storage against the given backend.
    pub fn open(config: MemoryConfig, storage: Arc<dyn Storage>) -> Self {
        let mem = Self {
            storage,
            entries: RwLock::new(HashMap::new()),
            index: RwLock::new(String::new()),
            config,
        };

        mem.migrate_legacy();
        mem.load_entries();
        mem.load_index();
        mem
    }

    fn load_entries(&self) {
        let keys = match self.storage.list(ENTRIES_PREFIX) {
            Ok(k) => k,
            Err(_) => return,
        };

        let mut entries = self.entries.write().unwrap();
        for key in keys {
            if !key.ends_with(".md") {
                continue;
            }
            let bytes = match self.storage.get(&key) {
                Ok(Some(b)) => b,
                _ => continue,
            };
            let raw = match std::str::from_utf8(&bytes) {
                Ok(s) => s,
                Err(_) => continue,
            };
            match MemoryEntry::parse(key.clone(), raw) {
                Ok(entry) => {
                    entries.insert(entry.name.clone(), entry);
                }
                Err(e) => {
                    tracing::warn!("failed to parse memory entry {key}: {e}");
                }
            }
        }
    }

    fn load_index(&self) {
        if let Ok(Some(bytes)) = self.storage.get(INDEX_KEY)
            && let Ok(content) = String::from_utf8(bytes)
        {
            *self.index.write().unwrap() = content;
        }
    }

    /// BM25-ranked recall over all entries.
    pub fn recall(&self, query: &str, limit: usize) -> String {
        let entries = self.entries.read().unwrap();
        if entries.is_empty() {
            return "no memories found".to_owned();
        }

        let entry_vec: Vec<&MemoryEntry> = entries.values().collect();
        let docs: Vec<(usize, String)> = entry_vec
            .iter()
            .enumerate()
            .map(|(i, e)| (i, e.search_text()))
            .collect();
        let doc_refs: Vec<(usize, &str)> = docs.iter().map(|(i, s)| (*i, s.as_str())).collect();

        let results = bm25::score(&doc_refs, query, limit);
        if results.is_empty() {
            return "no memories found".to_owned();
        }

        results
            .iter()
            .map(|(idx, _score)| {
                let e = &entry_vec[*idx];
                format!("## {}\n{}\n\n{}", e.name, e.description, e.content)
            })
            .collect::<Vec<_>>()
            .join("\n---\n")
    }

    /// Create or update a memory entry.
    pub fn remember(&self, name: String, description: String, content: String) -> String {
        let entry = MemoryEntry::new(name.clone(), description, content);
        if let Err(e) = entry.save(self.storage.as_ref()) {
            return format!("failed to save entry: {e}");
        }
        self.entries.write().unwrap().insert(name.clone(), entry);
        format!("remembered: {name}")
    }

    /// Delete a memory entry by name.
    pub fn forget(&self, name: &str) -> String {
        let mut entries = self.entries.write().unwrap();
        match entries.remove(name) {
            Some(entry) => {
                if let Err(e) = entry.delete(self.storage.as_ref()) {
                    tracing::warn!("failed to delete entry {}: {e}", entry.key);
                }
                format!("forgot: {name}")
            }
            None => format!("no entry named: {name}"),
        }
    }

    /// Overwrite MEMORY.md (the curated overview).
    pub fn write_index(&self, content: &str) -> String {
        if let Err(e) = self.storage.put(INDEX_KEY, content.as_bytes()) {
            return format!("failed to write MEMORY.md: {e}");
        }
        *self.index.write().unwrap() = content.to_owned();
        "MEMORY.md updated".to_owned()
    }

    /// Build system prompt block from MEMORY.md content.
    pub fn build_prompt(&self) -> String {
        let index = self.index.read().unwrap();
        if index.is_empty() {
            return format!("\n\n{MEMORY_PROMPT}");
        }
        format!("\n\n<memory>\n{}\n</memory>\n\n{MEMORY_PROMPT}", *index)
    }

    /// Auto-recall from last user message, injected before each turn.
    pub fn before_run(&self, history: &[HistoryEntry]) -> Vec<HistoryEntry> {
        let last_user = history
            .iter()
            .rev()
            .find(|e| *e.role() == Role::User && !e.text().is_empty());

        let Some(entry) = last_user else {
            return Vec::new();
        };

        let query: String = entry
            .text()
            .split_whitespace()
            .take(8)
            .collect::<Vec<_>>()
            .join(" ");

        if query.is_empty() {
            return Vec::new();
        }

        let limit = self.config.recall_limit;
        let result = self.recall(&query, limit);
        if result == "no memories found" {
            return Vec::new();
        }

        vec![HistoryEntry::user(format!("<recall>\n{result}\n</recall>")).auto_injected()]
    }

    /// Convert legacy memory.md/user.md/facts.toml blobs into the new
    /// per-entry layout. Runs only when the entries prefix is empty and at
    /// least one legacy key exists. Legacy keys are renamed to `.bak` on
    /// success so the migration is one-shot.
    fn migrate_legacy(&self) {
        let existing = self.storage.list(ENTRIES_PREFIX).unwrap_or_default();
        if !existing.is_empty() {
            return;
        }

        let legacy_memory = self.storage.get(LEGACY_MEMORY_KEY).ok().flatten();
        let legacy_user = self.storage.get(LEGACY_USER_KEY).ok().flatten();
        let legacy_facts = self.storage.get(LEGACY_FACTS_KEY).ok().flatten();

        if legacy_memory.is_none() && legacy_user.is_none() && legacy_facts.is_none() {
            return;
        }

        if let Some(bytes) = legacy_memory
            && let Ok(content) = String::from_utf8(bytes)
            && !content.trim().is_empty()
        {
            self.storage.put(INDEX_KEY, content.as_bytes()).ok();

            for (i, chunk) in content.split("\n\n").enumerate() {
                let chunk = chunk.trim();
                if chunk.is_empty() {
                    continue;
                }
                let name = format!("migrated-memory-{}", i + 1);
                let entry =
                    MemoryEntry::new(name, "Migrated from memory.md".to_owned(), chunk.to_owned());
                entry.save(self.storage.as_ref()).ok();
            }
            rename_key(self.storage.as_ref(), LEGACY_MEMORY_KEY, LEGACY_MEMORY_BAK);
        }

        if let Some(bytes) = legacy_user
            && let Ok(content) = String::from_utf8(bytes)
            && !content.trim().is_empty()
        {
            let entry = MemoryEntry::new(
                "user-profile".to_owned(),
                "User profile migrated from user.md".to_owned(),
                content,
            );
            entry.save(self.storage.as_ref()).ok();
            rename_key(self.storage.as_ref(), LEGACY_USER_KEY, LEGACY_USER_BAK);
        }

        if let Some(bytes) = legacy_facts
            && let Ok(content) = String::from_utf8(bytes)
            && !content.trim().is_empty()
        {
            let entry = MemoryEntry::new(
                "known-facts".to_owned(),
                "Known facts migrated from facts.toml".to_owned(),
                content,
            );
            entry.save(self.storage.as_ref()).ok();
            rename_key(self.storage.as_ref(), LEGACY_FACTS_KEY, LEGACY_FACTS_BAK);
        }
    }
}

/// The Storage trait has no `rename` — it's a KV store. The legacy
/// migration fakes it with copy+delete, which is fine for one-shot
/// backfill where atomicity doesn't matter.
fn rename_key(storage: &dyn Storage, from: &str, to: &str) {
    if let Ok(Some(bytes)) = storage.get(from) {
        if storage.put(to, &bytes).is_ok() {
            let _ = storage.delete(from);
        }
    }
}
