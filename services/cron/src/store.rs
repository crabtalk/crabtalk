//! TOML-backed schedule store — load, save, mutate `CronEntry` rows.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use wcore::trigger::cron::{CronEntry, validate_schedule};

/// TOML wrapper: `[[cron]]` array of tables.
#[derive(Debug, Default, Serialize, Deserialize)]
struct CronFile {
    #[serde(default)]
    cron: Vec<CronEntry>,
}

pub struct Store {
    path: PathBuf,
    entries: Vec<CronEntry>,
    next_id: u64,
}

impl Store {
    /// Load from `path`. Missing file is treated as empty.
    pub fn load(path: PathBuf) -> Result<Self> {
        let mut entries: Vec<CronEntry> = Vec::new();
        if let Ok(content) = std::fs::read_to_string(&path) {
            let file: CronFile = toml::from_str(&content)
                .with_context(|| format!("failed to parse {}", path.display()))?;
            for entry in file.cron {
                if let Err(e) = validate_schedule(&entry.schedule) {
                    tracing::warn!("cron {}: {e}, skipping", entry.id);
                    continue;
                }
                entries.push(entry);
            }
        }
        let next_id = entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        Ok(Self {
            path,
            entries,
            next_id,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn list(&self) -> &[CronEntry] {
        &self.entries
    }

    /// Insert a new entry, assigning an id. Validates the schedule.
    pub fn create(&mut self, mut entry: CronEntry) -> Result<&CronEntry> {
        validate_schedule(&entry.schedule).map_err(|e| anyhow::anyhow!("{e}"))?;
        entry.id = self.next_id;
        self.next_id += 1;
        self.entries.push(entry);
        self.save()?;
        Ok(self.entries.last().unwrap())
    }

    /// Remove by id. Returns true if an entry was removed.
    pub fn delete(&mut self, id: u64) -> Result<bool> {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        let removed = self.entries.len() != before;
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    /// Atomic write (tmp + rename).
    fn save(&self) -> Result<()> {
        let file = CronFile {
            cron: self.entries.clone(),
        };
        let content = toml::to_string_pretty(&file)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("toml.tmp");
        std::fs::write(&tmp, content)
            .with_context(|| format!("failed to write {}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path).with_context(|| {
            format!(
                "failed to rename {} → {}",
                tmp.display(),
                self.path.display()
            )
        })?;
        Ok(())
    }
}
