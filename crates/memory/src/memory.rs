use crate::{
    bm25::{Index, tokenize},
    entry::{Entry, EntryId, EntryKind},
    error::{Error, Result},
    file,
    op::Op,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Memory connection. `open(path)` is persistent (auto-flushes every
/// `apply` via atomic write); `new()` is in-RAM only.
pub struct Memory {
    path: Option<PathBuf>,
    entries: HashMap<EntryId, Entry>,
    by_name: HashMap<String, EntryId>,
    index: Index,
    next_id: EntryId,
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub entry: Entry,
    pub score: f64,
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

impl Memory {
    /// In-RAM memory. Nothing is persisted.
    pub fn new() -> Self {
        Self {
            path: None,
            entries: HashMap::new(),
            by_name: HashMap::new(),
            index: Index::new(),
            next_id: 1,
        }
    }

    /// Open (or create) a memory db at `path`. Reads the file if it
    /// exists; otherwise the db starts empty and the file is created on
    /// the first write.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut mem = Self {
            path: Some(path.clone()),
            entries: HashMap::new(),
            by_name: HashMap::new(),
            index: Index::new(),
            next_id: 1,
        };
        if let Some(snap) = file::read(&path)? {
            mem.next_id = snap.next_id;
            for entry in snap.entries {
                mem.by_name.insert(entry.name.clone(), entry.id);
                mem.reindex(&entry);
                mem.entries.insert(entry.id, entry);
            }
        }
        Ok(mem)
    }

    /// Apply a write op and persist. RAM is mutated before `flush`, so a
    /// flush failure leaves RAM ahead of disk until the next successful
    /// op (or the next `open`, which re-reads the file). WAL will close
    /// this window in v2.
    pub fn apply(&mut self, op: Op) -> Result<()> {
        match op {
            Op::Add {
                name,
                content,
                aliases,
                kind,
            } => self.add(name, content, aliases, kind)?,
            Op::Update {
                name,
                content,
                aliases,
            } => self.update(&name, content, aliases)?,
            Op::Alias { name, aliases } => self.set_aliases(&name, aliases)?,
            Op::Remove { name } => self.remove(&name)?,
        }
        self.flush()
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.by_name.get(name).and_then(|id| self.entries.get(id))
    }

    pub fn list(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        self.index
            .search(query, limit)
            .into_iter()
            .filter_map(|(id, score)| {
                self.entries.get(&id).map(|e| SearchHit {
                    entry: e.clone(),
                    score,
                })
            })
            .collect()
    }

    fn add(
        &mut self,
        name: String,
        content: String,
        aliases: Vec<String>,
        kind: EntryKind,
    ) -> Result<()> {
        if self.by_name.contains_key(&name) {
            return Err(Error::Duplicate(name));
        }
        let id = self.next_id;
        self.next_id += 1;
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let entry = Entry {
            id,
            name: name.clone(),
            content,
            aliases,
            created_at,
            kind,
        };
        self.reindex(&entry);
        self.by_name.insert(name, id);
        self.entries.insert(id, entry);
        Ok(())
    }

    fn update(&mut self, name: &str, content: String, aliases: Vec<String>) -> Result<()> {
        let id = *self
            .by_name
            .get(name)
            .ok_or_else(|| Error::NotFound(name.to_owned()))?;
        let entry = self.entries.get_mut(&id).expect("entry id out of sync");
        entry.content = content;
        entry.aliases = aliases;
        let snapshot = entry.clone();
        self.reindex(&snapshot);
        Ok(())
    }

    fn set_aliases(&mut self, name: &str, aliases: Vec<String>) -> Result<()> {
        let id = *self
            .by_name
            .get(name)
            .ok_or_else(|| Error::NotFound(name.to_owned()))?;
        let entry = self.entries.get_mut(&id).expect("entry id out of sync");
        entry.aliases = aliases;
        let snapshot = entry.clone();
        self.reindex(&snapshot);
        Ok(())
    }

    fn remove(&mut self, name: &str) -> Result<()> {
        let id = self
            .by_name
            .remove(name)
            .ok_or_else(|| Error::NotFound(name.to_owned()))?;
        self.entries.remove(&id);
        self.index.remove(id);
        Ok(())
    }

    fn reindex(&mut self, entry: &Entry) {
        let mut terms = tokenize(&entry.content);
        for alias in &entry.aliases {
            terms.extend(tokenize(alias));
        }
        self.index.insert(entry.id, &terms);
    }

    fn flush(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let mut entries: Vec<&Entry> = self.entries.values().collect();
        entries.sort_by_key(|e| e.id);
        file::write(path, self.next_id, &entries)
    }
}
