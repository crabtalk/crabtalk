//! Filesystem-backed [`SessionRepo`] implementation.
//!
//! Layout under `root`:
//! - `<slug>/meta` — JSON metadata blob.
//! - `<slug>/step-<NNNNNN>` — one file per persisted step.
//!
//! Slug format: `<agent>_<sender>_<seq>`, where `<seq>` is a monotonic
//! counter per (agent, sender) pair.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, io::ErrorKind, path::PathBuf, sync::Mutex};
use wcore::{
    ArchiveSegment, ConversationMeta, EventLine,
    model::HistoryEntry,
    repos::{SessionHandle, SessionRepo, SessionSnapshot, SessionSummary},
};

/// Width of zero-padded step counter.
const STEP_WIDTH: usize = 6;

/// One persisted step. Serialized as JSON.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum StepLine {
    Compact {
        compact: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        title: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        archived_at: String,
    },
    Event(EventLine),
    Entry(HistoryEntry),
}

pub struct FsSessionRepo {
    root: PathBuf,
    /// Per-session step counters, recovered from disk on first access.
    counters: Mutex<HashMap<String, u64>>,
}

impl FsSessionRepo {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            counters: Mutex::new(HashMap::new()),
        }
    }

    fn session_dir(&self, slug: &str) -> PathBuf {
        self.root.join(slug)
    }

    fn meta_path(&self, slug: &str) -> PathBuf {
        self.session_dir(slug).join("meta")
    }

    fn step_path(&self, slug: &str, step: u64) -> PathBuf {
        self.session_dir(slug)
            .join(format!("step-{step:0width$}", width = STEP_WIDTH))
    }

    /// Get the next step counter for a session, recovering from disk
    /// if not yet cached.
    fn next_step(&self, slug: &str) -> u64 {
        let mut counters = self.counters.lock().unwrap();
        let counter = counters
            .entry(slug.to_owned())
            .or_insert_with(|| recover_step_counter(&self.session_dir(slug)));
        let n = *counter;
        *counter += 1;
        n
    }

    fn write_step(&self, slug: &str, line: StepLine) -> Result<()> {
        let step = self.next_step(slug);
        let path = self.step_path(slug, step);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(&line)?;
        crate::repos::atomic_write(&path, &bytes)
    }
}

impl SessionRepo for FsSessionRepo {
    fn create(&self, agent: &str, created_by: &str) -> Result<SessionHandle> {
        let agent_slug = sender_slug(agent);
        let sender = sender_slug(created_by);
        let prefix = format!("{agent_slug}_{sender}_");
        let seq = next_session_seq(&self.root, &prefix);
        let slug = format!("{agent_slug}_{sender}_{seq}");

        let dir = self.session_dir(&slug);
        fs::create_dir_all(&dir)?;

        let meta = ConversationMeta {
            agent: agent.to_owned(),
            created_by: created_by.to_owned(),
            created_at: chrono::Utc::now().to_rfc3339(),
            title: String::new(),
            uptime_secs: 0,
        };
        let meta_bytes = serde_json::to_vec(&meta)?;
        crate::repos::atomic_write(&self.meta_path(&slug), &meta_bytes)?;

        Ok(SessionHandle::new(slug))
    }

    fn find_latest(&self, agent: &str, created_by: &str) -> Result<Option<SessionHandle>> {
        let agent_slug = sender_slug(agent);
        let sender = sender_slug(created_by);
        let prefix = format!("{agent_slug}_{sender}_");

        if !self.root.exists() {
            return Ok(None);
        }

        let mut best: Option<(u32, String)> = None;
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with(&prefix) || !entry.file_type()?.is_dir() {
                continue;
            }
            let seq_str = &name[prefix.len()..];
            if let Ok(seq) = seq_str.parse::<u32>()
                && best.as_ref().is_none_or(|(b, _)| seq > *b)
            {
                best = Some((seq, name.to_string()));
            }
        }
        Ok(best.map(|(_, slug)| SessionHandle::new(slug)))
    }

    fn load(&self, handle: &SessionHandle) -> Result<Option<SessionSnapshot>> {
        let slug = handle.as_str();
        let meta_path = self.meta_path(slug);
        let meta_bytes = match fs::read(&meta_path) {
            Ok(b) => b,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let meta: ConversationMeta = serde_json::from_slice(&meta_bytes)?;

        let dir = self.session_dir(slug);
        let mut step_files: Vec<_> = fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with("step-"))
            })
            .collect();
        step_files.sort_by_key(|e| e.file_name());

        let mut lines = Vec::with_capacity(step_files.len());
        let mut last_compact_idx: Option<usize> = None;
        for entry in &step_files {
            let bytes = fs::read(entry.path())?;
            match serde_json::from_slice::<StepLine>(&bytes) {
                Ok(line) => {
                    if matches!(line, StepLine::Compact { .. }) {
                        last_compact_idx = Some(lines.len());
                    }
                    lines.push(line);
                }
                Err(e) => {
                    tracing::warn!("skipping unparsable step {}: {e}", entry.path().display());
                }
            }
        }

        let start = last_compact_idx.unwrap_or(0);
        let mut history = Vec::new();
        for (i, line) in lines[start..].iter().enumerate() {
            match line {
                StepLine::Compact { compact, .. } if i == 0 && last_compact_idx.is_some() => {
                    history.push(HistoryEntry::user(compact));
                }
                StepLine::Entry(entry) => history.push(entry.clone()),
                StepLine::Event(_) | StepLine::Compact { .. } => {}
            }
        }

        Ok(Some(SessionSnapshot { meta, history }))
    }

    fn load_archives(&self, handle: &SessionHandle) -> Result<Vec<ArchiveSegment>> {
        let dir = self.session_dir(handle.as_str());
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut step_files: Vec<_> = fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with("step-"))
            })
            .collect();
        step_files.sort_by_key(|e| e.file_name());

        let mut archives = Vec::new();
        for entry in step_files {
            let bytes = fs::read(entry.path())?;
            if let Ok(StepLine::Compact {
                compact,
                title,
                archived_at,
            }) = serde_json::from_slice(&bytes)
            {
                archives.push(ArchiveSegment {
                    title,
                    summary: compact,
                    archived_at,
                });
            }
        }
        Ok(archives)
    }

    fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut summaries = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let slug = entry.file_name().to_string_lossy().to_string();
            let meta_path = self.meta_path(&slug);
            if let Ok(bytes) = fs::read(&meta_path)
                && let Ok(meta) = serde_json::from_slice::<ConversationMeta>(&bytes)
            {
                summaries.push(SessionSummary {
                    handle: SessionHandle::new(slug),
                    meta,
                });
            }
        }
        Ok(summaries)
    }

    fn append_messages(&self, handle: &SessionHandle, entries: &[HistoryEntry]) -> Result<()> {
        for entry in entries {
            self.write_step(handle.as_str(), StepLine::Entry(entry.clone()))?;
        }
        Ok(())
    }

    fn append_events(&self, handle: &SessionHandle, events: &[EventLine]) -> Result<()> {
        for event in events {
            self.write_step(handle.as_str(), StepLine::Event(event.clone()))?;
        }
        Ok(())
    }

    fn append_compact(&self, handle: &SessionHandle, summary: &str) -> Result<()> {
        let line = StepLine::Compact {
            compact: summary.to_owned(),
            title: compact_title(summary),
            archived_at: chrono::Utc::now().to_rfc3339(),
        };
        self.write_step(handle.as_str(), line)
    }

    fn update_meta(&self, handle: &SessionHandle, meta: &ConversationMeta) -> Result<()> {
        let path = self.meta_path(handle.as_str());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(meta)?;
        crate::repos::atomic_write(&path, &bytes)
    }

    fn delete(&self, handle: &SessionHandle) -> Result<bool> {
        let dir = self.session_dir(handle.as_str());
        match fs::remove_dir_all(&dir) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
}

/// Recover the step counter from the highest step-NNNNNN file.
fn recover_step_counter(dir: &PathBuf) -> u64 {
    let mut max = 0u64;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(suffix) = name.strip_prefix("step-")
                && let Ok(n) = suffix.parse::<u64>()
            {
                max = max.max(n);
            }
        }
    }
    max + 1
}

/// Find the next session sequence number.
fn next_session_seq(root: &PathBuf, prefix: &str) -> u32 {
    let mut max = 0u32;
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(seq_str) = name.strip_prefix(prefix)
                && let Ok(seq) = seq_str.parse::<u32>()
            {
                max = max.max(seq);
            }
        }
    }
    max + 1
}

fn sender_slug(s: &str) -> String {
    wcore::sender_slug(s)
}

fn compact_title(summary: &str) -> String {
    let end = summary
        .find(['.', '!', '?'])
        .map(|i| i + 1)
        .unwrap_or(summary.len())
        .min(60);
    let title = summary[..summary.floor_char_boundary(end)].trim();
    title.to_string()
}
