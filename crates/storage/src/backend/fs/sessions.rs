//! Session persistence — meta + append-only step files under
//! `sessions/<slug>/`. The on-disk step shape (`StepLine`) and
//! step-counter recovery live here too.

use crate::backend::fs::{FsStorage, atomic_write};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};
use tokio::fs;
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

impl FsStorage {
    fn session_dir(&self, slug: &str) -> PathBuf {
        self.sessions_root.join(slug)
    }

    fn session_meta_path(&self, slug: &str) -> PathBuf {
        self.session_dir(slug).join("meta")
    }

    fn session_step_path(&self, slug: &str, step: u64) -> PathBuf {
        self.session_dir(slug).join(format!("step-{step:06}"))
    }

    /// Reserve the next step number for a session. Holds the in-memory
    /// counter lock only across the synchronous `HashMap` lookup — disk
    /// recovery on first access happens outside the lock.
    async fn next_step(&self, slug: &str) -> u64 {
        {
            let mut counters = self.session_counters.lock();
            if let Some(counter) = counters.get_mut(slug) {
                let n = *counter;
                *counter += 1;
                return n;
            }
        }
        let recovered = recover_step_counter(&self.session_dir(slug)).await;
        let mut counters = self.session_counters.lock();
        let counter = counters.entry(slug.to_owned()).or_insert(recovered);
        let n = *counter;
        *counter += 1;
        n
    }

    async fn write_step(&self, slug: &str, line: StepLine) -> Result<()> {
        let step = self.next_step(slug).await;
        let path = self.session_step_path(slug, step);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let bytes = serde_json::to_vec(&line)?;
        atomic_write(&path, &bytes).await
    }

    pub(super) async fn create_session(
        &self,
        agent: &str,
        created_by: &str,
    ) -> Result<SessionHandle> {
        // Opaque identity. The directory name used to encode `(agent, sender)`,
        // which made both immutable in practice: renaming an agent orphaned its
        // transcripts, because the path could not follow. The association lives in
        // `meta` instead, where it can be rewritten.
        let slug = ulid::Ulid::new().to_string();

        let dir = self.session_dir(&slug);
        fs::create_dir_all(&dir).await?;

        let now = chrono::Utc::now().to_rfc3339();
        let meta = ConversationMeta {
            agent: agent.to_owned(),
            created_by: created_by.to_owned(),
            created_at: now.clone(),
            title: String::new(),
            updated_at: now,
            message_count: 0,
            summary: None,
        };
        let meta_bytes = serde_json::to_vec(&meta)?;
        atomic_write(&self.session_meta_path(&slug), &meta_bytes).await?;
        Ok(SessionHandle::new(slug))
    }

    /// The most recent session for an `(agent, created_by)` pair.
    ///
    /// Matches on `meta`, not on the directory name: an exact comparison, where the
    /// old prefix match conflated two agents whose names slugified alike. Ordered by
    /// `created_at` with the slug as tiebreak — the slug is opaque, so it breaks
    /// ties deterministically without being trusted to order on its own.
    pub(super) async fn find_latest_session(
        &self,
        agent: &str,
        created_by: &str,
    ) -> Result<Option<SessionHandle>> {
        let mut best: Option<(String, String)> = None;
        for summary in self.list_sessions().await? {
            if summary.meta.agent != agent || summary.meta.created_by != created_by {
                continue;
            }
            let key = (summary.meta.created_at, summary.handle.as_str().to_owned());
            if best.as_ref().is_none_or(|b| key > *b) {
                best = Some(key);
            }
        }
        Ok(best.map(|(_, slug)| SessionHandle::new(slug)))
    }

    pub(super) async fn load_session(
        &self,
        handle: &SessionHandle,
    ) -> Result<Option<SessionSnapshot>> {
        let slug = handle.as_str();
        let meta_path = self.session_meta_path(slug);
        let meta_bytes = match fs::read(&meta_path).await {
            Ok(b) => b,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let meta: ConversationMeta = serde_json::from_slice(&meta_bytes)?;

        let dir = self.session_dir(slug);
        let mut step_files: Vec<PathBuf> = Vec::new();
        let mut entries = fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry
                .file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("step-"))
            {
                step_files.push(entry.path());
            }
        }
        step_files.sort();

        let mut lines = Vec::with_capacity(step_files.len());
        let mut last_compact_idx: Option<usize> = None;
        for path in &step_files {
            let bytes = fs::read(path).await?;
            match serde_json::from_slice::<StepLine>(&bytes) {
                Ok(line) => {
                    if line.is_compact_boundary() {
                        last_compact_idx = Some(lines.len());
                    }
                    lines.push(line);
                }
                Err(e) => {
                    tracing::warn!("skipping unparsable step {}: {e}", path.display());
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

    pub(super) async fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        if !self.sessions_root.exists() {
            return Ok(Vec::new());
        }
        let mut summaries = Vec::new();
        let mut entries = fs::read_dir(&self.sessions_root).await?;
        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_dir() {
                continue;
            }
            let slug = entry.file_name().to_string_lossy().to_string();
            let meta_path = self.session_meta_path(&slug);
            if let Ok(bytes) = fs::read(&meta_path).await
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

    pub(super) async fn append_session_messages(
        &self,
        handle: &SessionHandle,
        entries: &[HistoryEntry],
    ) -> Result<()> {
        for entry in entries {
            self.write_step(handle.as_str(), StepLine::Entry(entry.clone()))
                .await?;
        }
        Ok(())
    }

    pub(super) async fn append_session_events(
        &self,
        handle: &SessionHandle,
        events: &[EventLine],
    ) -> Result<()> {
        for event in events {
            self.write_step(handle.as_str(), StepLine::Event(event.clone()))
                .await?;
        }
        Ok(())
    }

    pub(super) async fn append_session_compact(
        &self,
        handle: &SessionHandle,
        archive_name: &str,
    ) -> Result<()> {
        let line = StepLine::Compact {
            archive_name: archive_name.to_owned(),
            archived_at: chrono::Utc::now().to_rfc3339(),
        };
        self.write_step(handle.as_str(), line).await
    }

    pub(super) async fn truncate_session_messages(
        &self,
        handle: &SessionHandle,
        keep: usize,
    ) -> Result<()> {
        let Some(snapshot) = self.load_session(handle).await? else {
            return Ok(());
        };
        let kept: Vec<HistoryEntry> = snapshot.history.into_iter().take(keep).collect();
        // Wipe the step log, then re-lay the compacted boundary (if any) followed
        // by the kept entries. Trace events for the dropped turns go with them.
        // `next_step`'s counter keeps climbing, so new steps sort after the void.
        let dir = self.session_dir(handle.as_str());
        let mut rd = fs::read_dir(&dir).await?;
        while let Some(entry) = rd.next_entry().await? {
            if entry
                .file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("step-"))
            {
                fs::remove_file(entry.path()).await?;
            }
        }
        if let Some(archive_name) = snapshot.archive {
            self.write_step(
                handle.as_str(),
                StepLine::Compact {
                    archive_name,
                    archived_at: chrono::Utc::now().to_rfc3339(),
                },
            )
            .await?;
        }
        for entry in &kept {
            self.write_step(handle.as_str(), StepLine::Entry(entry.clone()))
                .await?;
        }
        let mut meta = snapshot.meta;
        meta.message_count = kept.len() as u64;
        self.update_session_meta(handle, &meta).await?;
        Ok(())
    }

    pub(super) async fn update_session_meta(
        &self,
        handle: &SessionHandle,
        meta: &ConversationMeta,
    ) -> Result<()> {
        let path = self.session_meta_path(handle.as_str());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let bytes = serde_json::to_vec(meta)?;
        atomic_write(&path, &bytes).await
    }

    pub(super) async fn delete_session(&self, handle: &SessionHandle) -> Result<bool> {
        let dir = self.session_dir(handle.as_str());
        match fs::remove_dir_all(&dir).await {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
}

async fn recover_step_counter(dir: &Path) -> u64 {
    let mut max = 0u64;
    if let Ok(mut entries) = fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
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
