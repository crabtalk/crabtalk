//! Conversation history: list, get, delete persisted sessions.

use crate::daemon::Daemon;
use anyhow::Result;
use crabllm_core::Provider;
use wcore::protocol::message::*;
use wcore::storage::Storage;

pub(super) async fn list_conversations<P: Provider + 'static>(
    node: &Daemon<P>,
    agent: String,
    sender: String,
) -> Result<Vec<ConversationInfo>> {
    let rt = node.runtime.read().await.clone();
    Ok(scan_sessions(rt.storage().as_ref(), &agent, &sender))
}

pub(super) async fn get_conversation_history<P: Provider + 'static>(
    node: &Daemon<P>,
    slug: String,
) -> Result<ConversationHistory> {
    let rt = node.runtime.read().await.clone();
    let handle = wcore::storage::SessionHandle::new(&slug);
    let snapshot = rt
        .storage()
        .load_session(&handle)?
        .ok_or_else(|| anyhow::anyhow!("conversation not found: {slug}"))?;
    let meta = snapshot.meta;
    let mut messages = snapshot.history;
    // Resolve the compacted prefix out of memory and prepend it, so the
    // UI sees the same pre-compact context the model does on resume.
    if let Some(name) = snapshot.archive {
        let content = {
            let mem = rt.memory().read();
            mem.get(&name).map(|e| e.content.clone())
        };
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

pub(super) async fn delete_conversation<P: Provider + 'static>(
    node: &Daemon<P>,
    slug: String,
) -> Result<()> {
    let rt = node.runtime.read().await.clone();
    let handle = wcore::storage::SessionHandle::new(&slug);
    let deleted = rt.storage().delete_session(&handle)?;
    if !deleted {
        anyhow::bail!("conversation not found: {slug}");
    }
    Ok(())
}

fn scan_sessions(storage: &impl Storage, agent: &str, sender: &str) -> Vec<ConversationInfo> {
    let Ok(summaries) = storage.list_sessions() else {
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
            message_count: 0,
            alive_secs: meta.uptime_secs,
            date: created_date_label(&meta.created_at),
        });
    }

    results.sort_by(|a, b| b.seq.cmp(&a.seq).then_with(|| a.agent.cmp(&b.agent)));
    results
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

fn created_date_label(created_at: &str) -> String {
    let Ok(ts) = chrono::DateTime::parse_from_rfc3339(created_at) else {
        return String::new();
    };
    let today = chrono::Local::now().date_naive();
    let date = ts.with_timezone(&chrono::Local).date_naive();
    if date == today {
        "Today".to_string()
    } else if date == today - chrono::Duration::days(1) {
        "Yesterday".to_string()
    } else {
        date.format("%Y-%m-%d").to_string()
    }
}
