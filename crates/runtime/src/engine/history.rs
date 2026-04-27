//! Conversation history queries — list, load, delete persisted sessions.

use super::Runtime;
use crate::Config;
use anyhow::Result;
use wcore::protocol::message::{ConversationHistory, ConversationInfo, ConversationMessage};
use wcore::storage::{SessionHandle, Storage};

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
                out.push(wcore::model::HistoryEntry::user(summary));
                out.append(&mut messages);
                messages = out;
            }
        }
        Ok(ConversationHistory {
            title: meta.title,
            agent: meta.agent,
            messages: messages
                .into_iter()
                .filter(|e| {
                    !matches!(
                        e.role(),
                        wcore::model::Role::System | wcore::model::Role::Tool
                    )
                })
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

    let agent_filter = if agent.is_empty() {
        None
    } else {
        Some(wcore::sender_slug(agent))
    };
    let sender_filter = if sender.is_empty() {
        None
    } else {
        Some(wcore::sender_slug(sender))
    };

    let mut results = Vec::new();
    for summary in summaries {
        let slug = summary.handle.as_str().to_owned();
        let meta = &summary.meta;
        let Some((slug_agent, slug_sender, seq)) = parse_session_slug(&slug) else {
            continue;
        };
        if let Some(ref want) = agent_filter
            && &slug_agent != want
        {
            continue;
        }
        if let Some(ref want) = sender_filter
            && &slug_sender != want
        {
            continue;
        }
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

fn parse_session_slug(slug: &str) -> Option<(String, String, u32)> {
    let parts: Vec<&str> = slug.split('_').collect();
    if parts.len() < 3 {
        return None;
    }
    let last = parts.len() - 1;
    if !parts[last].chars().all(|c| c.is_ascii_digit()) || parts[last].is_empty() {
        return None;
    }
    let seq: u32 = parts[last].parse().ok()?;
    let agent = parts[0].to_string();
    let sender = parts[1..last].join("_");
    Some((agent, sender, seq))
}
