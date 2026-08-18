//! Session history queries — list, load, delete persisted sessions.

use super::Runtime;
use crate::Config;
use anyhow::Result;
use crabllm_core::Role;
use proto::{ConversationHistory, ConversationInfo, ConversationMessage};
use std::collections::HashMap;
use store::{
    AgentId, SessionHandle,
    interface::{Memory, Sessions},
};

impl<C: Config> Runtime<C> {
    /// List persisted sessions, optionally filtered by agent and sender.
    /// Each entry carries the agent's name beside its id, because the
    /// listing is read by a person.
    pub async fn list_conversations(
        &self,
        agent: Option<AgentId>,
        sender: &str,
    ) -> Vec<ConversationInfo> {
        let Ok(summaries) = self.storage().list_sessions().await else {
            return Vec::new();
        };

        // Filtered on `meta`, the only place the association lives now that
        // the handle is opaque.
        let mut matched: Vec<_> = summaries
            .into_iter()
            .filter(|(_, meta)| agent.is_none_or(|id| meta.agent == id))
            .filter(|(_, meta)| sender.is_empty() || meta.created_by == sender)
            .collect();

        // `seq` is the nth session of an (agent, sender) pair. It used to be
        // read out of the path; it is derived here instead, by creation order
        // within the pair, so nothing has to store it.
        matched.sort_by(|(ah, am), (bh, bm)| {
            (&am.created_at, ah.as_str()).cmp(&(&bm.created_at, bh.as_str()))
        });
        // One read per agent, not per session — there are few of the
        // former and no bound on the latter.
        let names = self.agent_names().await;

        let mut seqs: HashMap<(AgentId, &str), u32> = HashMap::new();
        let mut results = Vec::with_capacity(matched.len());
        for (handle, meta) in &matched {
            let seq = seqs
                .entry((meta.agent, meta.created_by.as_str()))
                .and_modify(|n| *n += 1)
                .or_insert(1);
            results.push(ConversationInfo {
                agent_id: meta.agent.to_string(),
                agent_name: names.get(&meta.agent).cloned().unwrap_or_default(),
                sender: meta.created_by.clone(),
                seq: *seq,
                title: meta.title.clone(),
                file_path: handle.as_str().to_owned(),
                message_count: meta.message_count,
                // Wall-clock age between create and last update, in seconds.
                // 0 marks "unknown" (no `updated_at` in pre-0185 meta files).
                alive_secs: rfc3339_diff_secs(&meta.created_at, &meta.updated_at),
                // Raw RFC3339; callers format for display.
                date: meta.created_at.clone(),
            });
        }

        results.sort_by(|a, b| {
            b.seq
                .cmp(&a.seq)
                .then_with(|| a.agent_name.cmp(&b.agent_name))
        });
        results
    }

    /// Load a persisted session by slug, prepending the compacted archive
    /// (if any) so the UI sees the same pre-compact context the model does on
    /// resume.
    pub async fn load_conversation_history(&self, slug: &str) -> Result<ConversationHistory> {
        let handle = SessionHandle::new(slug);
        let snapshot = self
            .storage()
            .load_session(&handle)
            .await?
            .ok_or_else(|| anyhow::anyhow!("session not found: {slug}"))?;
        let meta = snapshot.meta;
        let mut messages = snapshot.history;
        if let Some(name) = snapshot.archive {
            let content = self.storage().memory(&name).await?.map(|e| e.content);
            if let Some(summary) = content {
                let mut out = Vec::with_capacity(messages.len() + 1);
                out.push(store::HistoryEntry::user(summary));
                out.append(&mut messages);
                messages = out;
            }
        }
        Ok(ConversationHistory {
            title: meta.title,
            agent_id: meta.agent.to_string(),
            agent_name: self
                .agent(&meta.agent)
                .await
                .map(|a| a.name)
                .unwrap_or_default(),
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

    /// Delete a persisted session by slug.
    pub async fn delete_conversation(&self, slug: &str) -> Result<()> {
        let handle = SessionHandle::new(slug);
        let deleted = self.storage().delete_session(&handle).await?;
        if !deleted {
            anyhow::bail!("session not found: {slug}");
        }
        Ok(())
    }
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
