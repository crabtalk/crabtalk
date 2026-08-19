//! Session persistence — resume, compaction, and message writes.

use super::Runtime;
use crate::{Config, Session, SessionHandle, SharedSession};
use anyhow::{Result, bail};
use crabllm_core::Role;
use store::{
    HistoryEntry, MemoryEntry,
    interface::{Memory, Sessions},
};
use tokio_util::sync::CancellationToken;

impl<C: Config> Runtime<C> {
    /// Rebuild a persisted session into a live one under `id`. `Ok(None)`
    /// if `handle` names no persisted session.
    ///
    /// Registering it is the caller's business; this only reads storage
    /// and replays the archived prefix.
    pub async fn load(&self, handle: SessionHandle, id: u64) -> Result<Option<Session>> {
        let Some(snapshot) = self.storage().load_session(&handle).await? else {
            return Ok(None);
        };
        if !self.has_agent(&snapshot.meta.agent).await {
            bail!("agent '{}' not found", snapshot.meta.agent);
        }
        let mut session = Session::new(id, &snapshot.meta.agent, &snapshot.meta.created_by);
        session.history = self
            .resumed_history(snapshot.archive.as_deref(), snapshot.history)
            .await;
        session.title = snapshot.meta.title;
        if !snapshot.meta.created_at.is_empty() {
            session.created_at_iso = snapshot.meta.created_at;
        }
        session.summary = snapshot.meta.summary;
        session.handle = Some(handle);
        Ok(Some(session))
    }

    /// Compact a session in-place: summarize history with the
    /// agent's compact LLM, write the summary to memory as an
    /// `Archive` entry, drop a compact marker into the session, and
    /// replace the live history with a single user message carrying
    /// the summary. `prompt` is the caller-supplied summarization
    /// instruction. Returns the summary on success, or `None` if
    /// `cancel` fires before the summary comes back.
    pub async fn compact(
        &self,
        session: &SharedSession,
        prompt: &str,
        cancel: Option<CancellationToken>,
    ) -> Option<String> {
        // Held across the summarizing round trip, the way a run holds it.
        // Releasing it for the call would let a send land messages the
        // summary does not cover, which the truncation below then deletes.
        if prompt.is_empty() {
            return None;
        }
        let mut session = session.lock().await;
        if session.history.is_empty() {
            return None;
        }
        let agent = self.resolve_agent(&session.agent).await?;
        let summary = agent.compact(&session.history, prompt, cancel).await?;

        let handle = session.handle.clone()?;
        let archive_name = self.write_archive(handle.as_str(), summary.clone()).await?;

        let storage = self.storage();
        if let Err(e) = storage.append_session_compact(&handle, &archive_name).await {
            tracing::warn!("compact: marker write failed: {e}");
            return None;
        }
        session.history = vec![HistoryEntry::user(&summary)];
        session.summary = Some(summary.clone());
        let _ = storage.update_session_meta(&handle, &session.meta()).await;
        Some(summary)
    }

    /// Rewind a session by dropping its last `turns` user turns — the
    /// user message that opens each turn and everything after it — from both
    /// the live history and storage. Used to re-run an edited message: drop
    /// the turn, then re-send the new text. Never rewinds past a compacted
    /// prefix. Returns the new history length.
    pub async fn truncate(&self, session: &SharedSession, turns: usize) -> Result<usize> {
        let mut session = session.lock().await;
        // Boundary = the index of the `turns`-th user entry counting from the
        // end; everything before it survives.
        let mut seen = 0;
        let mut boundary = None;
        for (i, entry) in session.history.iter().enumerate().rev() {
            if matches!(entry.role(), Role::User) && !entry.auto_injected {
                seen += 1;
                if seen == turns {
                    boundary = Some(i);
                    break;
                }
            }
        }
        let Some(boundary) = boundary else {
            return Ok(session.history.len());
        };
        session.history.truncate(boundary);
        if let Some(handle) = session.handle.clone() {
            // The storage layer's history excludes the archive prefix, so
            // `keep` counts the post-compact, non-injected survivors.
            let archive_offset = usize::from(session.summary.is_some());
            let keep = session.history[archive_offset..]
                .iter()
                .filter(|e| !e.auto_injected)
                .count();
            self.storage()
                .truncate_session_messages(&handle, keep)
                .await?;
        }
        Ok(session.history.len())
    }

    /// Build the session's replay history from storage's post-compact
    /// messages plus the Archive entry's content. A missing archive entry
    /// (memory wiped, different machine, etc.) injects a visible placeholder
    /// so the model can acknowledge the gap instead of silently truncating
    /// the user's context.
    async fn resumed_history(
        &self,
        archive: Option<&str>,
        mut history: Vec<HistoryEntry>,
    ) -> Vec<HistoryEntry> {
        let Some(name) = archive else { return history };
        let content = self
            .storage()
            .memory(name)
            .await
            .ok()
            .flatten()
            .map(|e| e.content);
        let prefix = content.unwrap_or_else(|| {
            tracing::warn!("resume: archive '{name}' missing from memory");
            format!("[archived context unavailable: {name}]")
        });
        let mut out = Vec::with_capacity(history.len() + 1);
        out.push(HistoryEntry::user(prefix));
        out.append(&mut history);
        out
    }

    /// Write a compaction summary to memory as an `Archive` entry,
    /// named `{session-slug}-{n}` where `n` is the next free sequence
    /// number for this session. Older archives stay searchable via
    /// `recall`, so a long-running session's phases don't get
    /// overwritten. Returns the generated name, or `None` on failure
    /// — the caller must skip the compact marker so a resume can't
    /// dangle.
    async fn write_archive(&self, session_slug: &str, summary: String) -> Option<String> {
        let slug = store::sender_slug(session_slug);
        let prefix = format!("{slug}-");
        // The sequence is derived from the names already stored, so two
        // compactions racing can pick the same one. The loser overwrites
        // rather than corrupting: an archive is write-once content keyed
        // by a name, and both hold the same session's summary.
        let next_seq = self
            .storage()
            .memory_names_under(&prefix)
            .await
            .unwrap_or_default()
            .iter()
            .filter_map(|name| {
                let suffix = &name[prefix.len()..];
                let n: u32 = suffix.parse().ok()?;
                // Reject non-canonical forms ("02", "+1", etc.) so a
                // future `{slug}-2` can't collide with a historic
                // `{slug}-02`.
                (n.to_string() == suffix).then_some(n)
            })
            .max()
            .unwrap_or(0)
            + 1;
        let name = format!("{slug}-{next_seq}");
        let entry = MemoryEntry {
            name: name.clone(),
            kind: "archive".to_owned(),
            content: summary,
            aliases: Vec::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        match self.storage().put_memory(&entry).await {
            Ok(()) => Some(name),
            Err(e) => {
                tracing::error!("archive write failed: {e}");
                None
            }
        }
    }

    /// Post-run tail shared by `send_to`, `stream_to`, and
    /// `guest_stream_to`: persist messages and event trace. Also threads
    /// each newly persisted entry into the session search index so
    /// `search_sessions` finds live work without waiting for a process
    /// restart.
    ///
    /// Every live session already has a handle — `Sessions::open`
    /// creates or loads one before registering it — so the guard below
    /// is a defensive no-op, not the normal path.
    pub(crate) async fn finalize_run(
        &self,
        session: &mut Session,
        pre_run_len: usize,
        event_trace: &[store::EventLine],
    ) {
        let Some(ref handle) = session.handle else {
            return;
        };
        let storage = self.storage();

        let new_entries: Vec<_> = session.history[pre_run_len..]
            .iter()
            .filter(|e| !e.auto_injected)
            .cloned()
            .collect();
        let _ = storage.append_session_messages(handle, &new_entries).await;
        if !event_trace.is_empty() {
            let _ = storage.append_session_events(handle, event_trace).await;
        }
        let meta = session.meta();
        let _ = storage.update_session_meta(handle, &meta).await;
    }
}
