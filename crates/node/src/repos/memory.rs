//! Filesystem-backed [`MemoryRepo`] implementation.
//!
//! Layout under `root`:
//! - `entries/<slug>.md` — one file per entry (frontmatter + body).
//! - `MEMORY.md` — curated index.

use anyhow::Result;
use std::{fs, io::ErrorKind, path::PathBuf};
use wcore::repos::{MemoryEntry, MemoryRepo, slugify};

pub struct FsMemoryRepo {
    root: PathBuf,
}

impl FsMemoryRepo {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn entries_dir(&self) -> PathBuf {
        self.root.join("entries")
    }

    fn entry_path(&self, name: &str) -> PathBuf {
        self.entries_dir().join(format!("{}.md", slugify(name)))
    }

    fn index_path(&self) -> PathBuf {
        self.root.join("MEMORY.md")
    }
}

impl MemoryRepo for FsMemoryRepo {
    fn list(&self) -> Result<Vec<MemoryEntry>> {
        let dir = self.entries_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut entries = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md") {
                let content = fs::read_to_string(&path)?;
                match MemoryEntry::parse(&content) {
                    Ok(parsed) => entries.push(parsed),
                    Err(e) => {
                        tracing::warn!("failed to parse {}: {e}", path.display());
                    }
                }
            }
        }
        Ok(entries)
    }

    fn load(&self, name: &str) -> Result<Option<MemoryEntry>> {
        let path = self.entry_path(name);
        match fs::read_to_string(&path) {
            Ok(content) => Ok(Some(MemoryEntry::parse(&content)?)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn save(&self, entry: &MemoryEntry) -> Result<()> {
        let path = self.entry_path(&entry.name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        crate::repos::atomic_write(&path, entry.serialize().as_bytes())
    }

    fn delete(&self, name: &str) -> Result<bool> {
        let path = self.entry_path(name);
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    fn load_index(&self) -> Result<Option<String>> {
        let path = self.index_path();
        match fs::read_to_string(&path) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn save_index(&self, content: &str) -> Result<()> {
        let path = self.index_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        crate::repos::atomic_write(&path, content.as_bytes())
    }
}
