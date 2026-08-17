//! Conversation history queries — list, load, delete persisted sessions.

use super::Runtime;
use crate::Config;
use anyhow::Result;
use crabllm_core::Role;
use proto::{ConversationHistory, ConversationInfo, ConversationMessage};
use std::collections::HashMap;
use storage::{SessionHandle, SessionSummary, Storage};

impl<C: Config> Runtime<C> {
    /// List persisted conversations, optionally filtered by agent and sender.
    pub async fn list_conversations(&self, agent: &str, sender: &str) -> Vec<ConversationInfo> {
        scan_sessions(self.storage().as_ref(), agent, sender).await
    }

    /// Load a persisted conversation by slug, prepending the compacted archive
    /// (if any) so the UI sees the same pre-compact context the model does on
    /// resume.
    pub async fn load_conversation_history(&self, slug: &str) -> Result<ConversationHistory> {
        let handle = SessionHandle::new(slug);
        let snapshot = self
            .storage()
            .load_session(&handle)
            .await?
            .ok_or_else(|| anyhow::anyhow!("conversation not found: {slug}"))?;
        let meta = snapshot.meta;
        let mut messages = snapshot.history;
        if let Some(name) = snapshot.archive {
            let content = self.memory().read().get(&name).map(|e| e.content.clone());
            if let Some(summary) = content {
                let mut out = Vec::with_capacity(messages.len() + 1);
                out.push(storage::HistoryEntry::user(summary));
                out.append(&mut messages);
                messages = out;
            }
        }
        Ok(ConversationHistory {
            title: meta.title,
            agent: meta.agent,
            messages: messages
                .into_iter()
                .filter(|e| !matches!(e.role(), Role::System | Role::Tool))
                .map(|e| ConversationMessage {
                    role: e.role().as_str().to_owned(),
                    content: e.text().to_owned(),
                })
                .collect(),
        })
    }

    /// Delete a persisted conversation by slug.
    pub async fn delete_conversation(&self, slug: &str) -> Result<()> {
        let handle = SessionHandle::new(slug);
        let deleted = self.storage().delete_session(&handle).await?;
        if !deleted {
            anyhow::bail!("conversation not found: {slug}");
        }
        Ok(())
    }
}

async fn scan_sessions(storage: &impl Storage, agent: &str, sender: &str) -> Vec<ConversationInfo> {
    let Ok(summaries) = storage.list_sessions().await else {
        return Vec::new();
    };

    // Filtered on `meta`, the only place the association lives now that the
    // directory name is opaque. Also an exact match, where slugifying both sides
    // conflated agents whose names differ only in punctuation.
    let mut matched: Vec<_> = summaries
        .into_iter()
        .filter(|s| agent.is_empty() || s.meta.agent == agent)
        .filter(|s| sender.is_empty() || s.meta.created_by == sender)
        .collect();

    // `seq` is the nth conversation of an (agent, sender) pair. It used to be
    // read out of the path; it is derived here instead, by creation order within
    // the pair, so nothing has to store it.
    matched.sort_by(|a, b| {
        (&a.meta.created_at, a.handle.as_str()).cmp(&(&b.meta.created_at, b.handle.as_str()))
    });
    let mut seqs: HashMap<(&str, &str), u32> = HashMap::new();
    let numbered: Vec<(u32, &SessionSummary)> = matched
        .iter()
        .map(|s| {
            let seq = seqs
                .entry((s.meta.agent.as_str(), s.meta.created_by.as_str()))
                .and_modify(|n| *n += 1)
                .or_insert(1);
            (*seq, s)
        })
        .collect();

    let mut results = Vec::new();
    for (seq, summary) in numbered {
        let slug = summary.handle.as_str().to_owned();
        let meta = &summary.meta;
        results.push(ConversationInfo {
            agent: meta.agent.clone(),
            sender: meta.created_by.clone(),
            seq,
            title: meta.title.clone(),
            file_path: slug,
            message_count: meta.message_count,
            // Wall-clock age between create and last update, in seconds.
            // 0 marks "unknown" (no `updated_at` in pre-0185 meta files).
            alive_secs: rfc3339_diff_secs(&meta.created_at, &meta.updated_at),
            // Raw RFC3339; callers format for display.
            date: meta.created_at.clone(),
        });
    }

    results.sort_by(|a, b| b.seq.cmp(&a.seq).then_with(|| a.agent.cmp(&b.agent)));
    results
}

/// Wall-clock seconds between two RFC3339 timestamps. Returns 0 if
/// either is empty (pre-0185 meta lines have no `updated_at`) or if
/// parsing fails — callers display 0 as "unknown."
fn rfc3339_diff_secs(start: &str, end: &str) -> u64 {
    if start.is_empty() || end.is_empty() {
        return 0;
    }
    let Ok(s) = chrono::DateTime::parse_from_rfc3339(start) else {
        return 0;
    };
    let Ok(e) = chrono::DateTime::parse_from_rfc3339(end) else {
        return 0;
    };
    (e - s).num_seconds().max(0) as u64
}
