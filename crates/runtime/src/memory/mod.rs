//! Built-in memory — file-per-entry storage at `{config_dir}/memory/`.

use crate::config::MemoryConfig;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::RwLock,
};
use wcore::model::{Message, Role};

pub mod bm25;
pub mod entry;
pub mod storage;
pub mod tool;

use entry::MemoryEntry;
use storage::Storage;

const MEMORY_PROMPT: &str = include_str!("../../prompts/memory.md");

pub const DEFAULT_SOUL: &str = include_str!("../../prompts/crab.md");

pub struct Memory {
    storage: Box<dyn Storage>,
    entries: RwLock<HashMap<String, MemoryEntry>>,
    index: RwLock<String>,
    index_path: PathBuf,
    entries_dir: PathBuf,
    config: MemoryConfig,
}

impl Memory {
    /// Open (or create) memory storage at the given directory.
    pub fn open(dir: PathBuf, config: MemoryConfig, storage: Box<dyn Storage>) -> Self {
        let entries_dir = dir.join("entries");
        let index_path = dir.join("MEMORY.md");

        storage.create_dir_all(&entries_dir).ok();

        let mem = Self {
            storage,
            entries: RwLock::new(HashMap::new()),
            index: RwLock::new(String::new()),
            index_path,
            entries_dir,
            config,
        };

        mem.migrate_legacy(&dir);
        mem.load_entries();
        mem.load_index();
        mem
    }

    fn load_entries(&self) {
        let paths = match self.storage.list(&self.entries_dir) {
            Ok(p) => p,
            Err(_) => return,
        };

        let mut entries = self.entries.write().unwrap();
        for path in paths {
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let raw = match self.storage.read(&path) {
                Ok(r) => r,
                Err(_) => continue,
            };
            match MemoryEntry::parse(path, &raw) {
                Ok(entry) => {
                    entries.insert(entry.name.clone(), entry);
                }
                Err(e) => {
                    tracing::warn!("failed to parse memory entry: {e}");
                }
            }
        }
    }

    fn load_index(&self) {
        if let Ok(content) = self.storage.read(&self.index_path) {
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
        let entry = MemoryEntry::new(name.clone(), description, content, &self.entries_dir);
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
                    tracing::warn!("failed to delete entry file: {e}");
                }
                format!("forgot: {name}")
            }
            None => format!("no entry named: {name}"),
        }
    }

    /// Overwrite MEMORY.md (the curated overview).
    pub fn write_index(&self, content: &str) -> String {
        if let Err(e) = self.storage.write(&self.index_path, content) {
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
    pub fn before_run(&self, history: &[Message]) -> Vec<Message> {
        let last_user = history
            .iter()
            .rev()
            .find(|m| m.role == Role::User && !m.content.is_empty());

        let Some(msg) = last_user else {
            return Vec::new();
        };

        let query: String = msg
            .content
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

        vec![Message {
            role: Role::User,
            content: format!("<recall>\n{result}\n</recall>"),
            auto_injected: true,
            ..Default::default()
        }]
    }

    fn migrate_legacy(&self, dir: &Path) {
        let existing = self.storage.list(&self.entries_dir).unwrap_or_default();
        if !existing.is_empty() {
            return;
        }

        let memory_path = dir.join("memory.md");
        let user_path = dir.join("user.md");
        let facts_path = dir.join("facts.toml");

        let has_legacy = self.storage.exists(&memory_path)
            || self.storage.exists(&user_path)
            || self.storage.exists(&facts_path);

        if !has_legacy {
            return;
        }

        if let Ok(content) = self.storage.read(&memory_path)
            && !content.trim().is_empty()
        {
            self.storage.write(&self.index_path, &content).ok();

            for (i, chunk) in content.split("\n\n").enumerate() {
                let chunk = chunk.trim();
                if chunk.is_empty() {
                    continue;
                }
                let name = format!("migrated-memory-{}", i + 1);
                let entry = MemoryEntry::new(
                    name,
                    "Migrated from memory.md".to_owned(),
                    chunk.to_owned(),
                    &self.entries_dir,
                );
                entry.save(self.storage.as_ref()).ok();
            }
            self.storage
                .rename(&memory_path, &dir.join("memory.md.bak"))
                .ok();
        }

        if let Ok(content) = self.storage.read(&user_path)
            && !content.trim().is_empty()
        {
            let entry = MemoryEntry::new(
                "user-profile".to_owned(),
                "User profile migrated from user.md".to_owned(),
                content,
                &self.entries_dir,
            );
            entry.save(self.storage.as_ref()).ok();
            self.storage
                .rename(&user_path, &dir.join("user.md.bak"))
                .ok();
        }

        if let Ok(content) = self.storage.read(&facts_path)
            && !content.trim().is_empty()
        {
            let entry = MemoryEntry::new(
                "known-facts".to_owned(),
                "Known facts migrated from facts.toml".to_owned(),
                content,
                &self.entries_dir,
            );
            entry.save(self.storage.as_ref()).ok();
            self.storage
                .rename(&facts_path, &dir.join("facts.toml.bak"))
                .ok();
        }
    }
}
