//! Built-in memory — cached access to a [`MemoryRepo`] backend.
//!
//! `Memory` wraps a `MemoryRepo` with an in-process entry cache (for
//! BM25 recall scoring) and the MEMORY.md index. The repo owns the
//! physical layout; Memory owns search and prompt generation.

use crate::config::MemoryConfig;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use wcore::{
    model::{HistoryEntry, Role},
    repos::{MemoryEntry, MemoryRepo},
};

pub mod bm25;
pub mod entry;
pub mod tool;

const MEMORY_PROMPT: &str = include_str!("../../prompts/memory.md");

pub const DEFAULT_SOUL: &str = include_str!("../../prompts/crab.md");

pub struct Memory<M: MemoryRepo> {
    repo: Arc<M>,
    entries: RwLock<HashMap<String, MemoryEntry>>,
    index: RwLock<String>,
    config: MemoryConfig,
}

impl<M: MemoryRepo> Memory<M> {
    /// Open memory storage against the given repo backend.
    pub fn open(config: MemoryConfig, repo: Arc<M>) -> Self {
        let mem = Self {
            repo,
            entries: RwLock::new(HashMap::new()),
            index: RwLock::new(String::new()),
            config,
        };

        mem.load_entries();
        mem.load_index();
        mem
    }

    fn load_entries(&self) {
        let loaded = match self.repo.list() {
            Ok(entries) => entries,
            Err(_) => return,
        };
        let mut cache = self.entries.write().unwrap();
        for entry in loaded {
            cache.insert(entry.name.clone(), entry);
        }
    }

    fn load_index(&self) {
        if let Ok(Some(content)) = self.repo.load_index() {
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
        let entry = MemoryEntry {
            name: name.clone(),
            description,
            content,
        };
        if let Err(e) = self.repo.save(&entry) {
            return format!("failed to save entry: {e}");
        }
        self.entries.write().unwrap().insert(name.clone(), entry);
        format!("remembered: {name}")
    }

    /// Delete a memory entry by name.
    pub fn forget(&self, name: &str) -> String {
        let mut entries = self.entries.write().unwrap();
        match entries.remove(name) {
            Some(_) => {
                if let Err(e) = self.repo.delete(name) {
                    tracing::warn!("failed to delete memory entry '{name}': {e}");
                }
                format!("forgot: {name}")
            }
            None => format!("no entry named: {name}"),
        }
    }

    /// Overwrite MEMORY.md (the curated overview).
    pub fn write_index(&self, content: &str) -> String {
        if let Err(e) = self.repo.save_index(content) {
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
}
