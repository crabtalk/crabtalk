//! Session persistence — meta + append-only step files under
//! `sessions/<slug>/`. The on-disk step shape (`StepLine`) and
//! step-counter recovery live here too.

use super::{FsStorage, atomic_write};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};
use wcore::{
    ConversationMeta, EventLine,
    model::HistoryEntry,
    storage::{SessionHandle, SessionSnapshot, SessionSummary},
};

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum StepLine {
    Compact {
        /// Name of the `Archive`-kind entry in `memory` whose content
        /// is the compacted prefix of the session up to this point.
        archive_name: String,
        archived_at: String,
    },
    /// Pre-Phase-5 compact marker that stored the summary inline.
    /// Still recognized on read so older sessions keep their replay
    /// boundary; the inline summary is no longer available, but the
    /// boundary itself prevents stale pre-compact history from being
    /// replayed.
    LegacyCompact {
        compact: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        archived_at: String,
    },
    Event(EventLine),
    Entry(HistoryEntry),
}

impl StepLine {
    fn is_compact_boundary(&self) -> bool {
        matches!(self, Self::Compact { .. } | Self::LegacyCompact { .. })
    }
}

fn session_dir(storage: &FsStorage, slug: &str) -> PathBuf {
    storage.sessions_root.join(slug)
}

fn session_meta_path(storage: &FsStorage, slug: &str) -> PathBuf {
    session_dir(storage, slug).join("meta")
}

fn session_step_path(storage: &FsStorage, slug: &str, step: u64) -> PathBuf {
    session_dir(storage, slug).join(format!("step-{step:06}"))
}

fn next_step(storage: &FsStorage, slug: &str) -> u64 {
    let mut counters = storage.session_counters.lock();
    let counter = counters
        .entry(slug.to_owned())
        .or_insert_with(|| recover_step_counter(&session_dir(storage, slug)));
    let n = *counter;
    *counter += 1;
    n
}

fn write_step(storage: &FsStorage, slug: &str, line: StepLine) -> Result<()> {
    let step = next_step(storage, slug);
    let path = session_step_path(storage, slug, step);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec(&line)?;
    atomic_write(&path, &bytes)
}

pub(super) fn create_session(
    storage: &FsStorage,
    agent: &str,
    created_by: &str,
) -> Result<SessionHandle> {
    let agent_slug = wcore::sender_slug(agent);
    let sender = wcore::sender_slug(created_by);
    let prefix = format!("{agent_slug}_{sender}_");
    let seq = next_session_seq(&storage.sessions_root, &prefix);
    let slug = format!("{agent_slug}_{sender}_{seq}");

    let dir = session_dir(storage, &slug);
    fs::create_dir_all(&dir)?;

    let meta = ConversationMeta {
        agent: agent.to_owned(),
        created_by: created_by.to_owned(),
        created_at: chrono::Utc::now().to_rfc3339(),
        title: String::new(),
        uptime_secs: 0,
        topic: None,
    };
    let meta_bytes = serde_json::to_vec(&meta)?;
    atomic_write(&session_meta_path(storage, &slug), &meta_bytes)?;
    Ok(SessionHandle::new(slug))
}

pub(super) fn find_latest_session(
    storage: &FsStorage,
    agent: &str,
    created_by: &str,
) -> Result<Option<SessionHandle>> {
    let agent_slug = wcore::sender_slug(agent);
    let sender = wcore::sender_slug(created_by);
    let prefix = format!("{agent_slug}_{sender}_");

    if !storage.sessions_root.exists() {
        return Ok(None);
    }

    let mut best: Option<(u32, String)> = None;
    for entry in fs::read_dir(&storage.sessions_root)? {
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

pub(super) fn load_session(
    storage: &FsStorage,
    handle: &SessionHandle,
) -> Result<Option<SessionSnapshot>> {
    let slug = handle.as_str();
    let meta_path = session_meta_path(storage, slug);
    let meta_bytes = match fs::read(&meta_path) {
        Ok(b) => b,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let meta: ConversationMeta = serde_json::from_slice(&meta_bytes)?;

    let dir = session_dir(storage, slug);
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
                if line.is_compact_boundary() {
                    last_compact_idx = Some(lines.len());
                }
                lines.push(line);
            }
            Err(e) => {
                tracing::warn!("skipping unparsable step {}: {e}", entry.path().display());
            }
        }
    }

    // If a compact boundary was seen, replay starts at it: the first
    // line in this slice is that boundary, and we lift its archive
    // name out before walking the rest.
    let start = last_compact_idx.unwrap_or(0);
    let resume_after_compact = last_compact_idx.is_some();
    let mut history = Vec::new();
    let mut archive = None;
    for (i, line) in lines[start..].iter().enumerate() {
        let is_resume_boundary = resume_after_compact && i == 0;
        match line {
            StepLine::Compact { archive_name, .. } if is_resume_boundary => {
                archive = Some(archive_name.clone());
            }
            StepLine::Entry(entry) => history.push(entry.clone()),
            StepLine::Event(_) | StepLine::Compact { .. } | StepLine::LegacyCompact { .. } => {}
        }
    }

    Ok(Some(SessionSnapshot {
        meta,
        history,
        archive,
    }))
}

pub(super) fn list_sessions(storage: &FsStorage) -> Result<Vec<SessionSummary>> {
    if !storage.sessions_root.exists() {
        return Ok(Vec::new());
    }
    let mut summaries = Vec::new();
    for entry in fs::read_dir(&storage.sessions_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let slug = entry.file_name().to_string_lossy().to_string();
        let meta_path = session_meta_path(storage, &slug);
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

pub(super) fn append_session_messages(
    storage: &FsStorage,
    handle: &SessionHandle,
    entries: &[HistoryEntry],
) -> Result<()> {
    for entry in entries {
        write_step(storage, handle.as_str(), StepLine::Entry(entry.clone()))?;
    }
    Ok(())
}

pub(super) fn append_session_events(
    storage: &FsStorage,
    handle: &SessionHandle,
    events: &[EventLine],
) -> Result<()> {
    for event in events {
        write_step(storage, handle.as_str(), StepLine::Event(event.clone()))?;
    }
    Ok(())
}

pub(super) fn append_session_compact(
    storage: &FsStorage,
    handle: &SessionHandle,
    archive_name: &str,
) -> Result<()> {
    let line = StepLine::Compact {
        archive_name: archive_name.to_owned(),
        archived_at: chrono::Utc::now().to_rfc3339(),
    };
    write_step(storage, handle.as_str(), line)
}

pub(super) fn update_session_meta(
    storage: &FsStorage,
    handle: &SessionHandle,
    meta: &ConversationMeta,
) -> Result<()> {
    let path = session_meta_path(storage, handle.as_str());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec(meta)?;
    atomic_write(&path, &bytes)
}

pub(super) fn delete_session(storage: &FsStorage, handle: &SessionHandle) -> Result<bool> {
    let dir = session_dir(storage, handle.as_str());
    match fs::remove_dir_all(&dir) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

fn recover_step_counter(dir: &Path) -> u64 {
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

fn next_session_seq(root: &Path, prefix: &str) -> u32 {
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
