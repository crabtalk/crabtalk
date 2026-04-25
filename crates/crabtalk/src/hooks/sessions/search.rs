//! `search_sessions` — BM25 over indexed conversation messages,
//! returns ranked excerpts with bounded context windows.

use super::SessionsHook;
use crabllm_core::Provider;
use runtime::sessions::{SearchOptions, SessionHit, WindowItem};
use schemars::JsonSchema;
use serde::Deserialize;
use wcore::ToolDispatch;

/// Search past conversation messages by keyword. Returns ranked
/// excerpts (matched message + surrounding context) — never full
/// sessions. Use the returned session handle to drill in further.
#[derive(Deserialize, JsonSchema)]
pub struct SearchSessions {
    /// Keyword or phrase to match against message content.
    pub query: String,
    /// Maximum number of session hits to return. Defaults to 5;
    /// clamped to the index's hard cap.
    pub limit: Option<usize>,
    /// Messages to include before each match. Defaults to 4.
    pub context_before: Option<usize>,
    /// Messages to include after each match. Defaults to 4.
    pub context_after: Option<usize>,
    /// Restrict to sessions for this agent name.
    pub agent: Option<String>,
    /// Restrict to sessions started by this sender.
    pub sender: Option<String>,
}

impl<P: Provider + 'static> SessionsHook<P> {
    pub(super) async fn handle_search_sessions(
        &self,
        call: ToolDispatch,
    ) -> Result<String, String> {
        let input: SearchSessions =
            serde_json::from_str(&call.args).map_err(|e| format!("invalid arguments: {e}"))?;
        let shared = self
            .runtime
            .get()
            .ok_or_else(|| "search_sessions: runtime not initialized".to_owned())?;
        let rt = shared.read().await.clone();
        let opts = SearchOptions {
            limit: input.limit.unwrap_or(5),
            context_before: input.context_before.unwrap_or(4),
            context_after: input.context_after.unwrap_or(4),
            agent_filter: input.agent,
            sender_filter: input.sender,
        };
        let hits = rt.search_sessions(&input.query, &opts);
        Ok(format_hits(&hits))
    }
}

fn format_hits(hits: &[SessionHit]) -> String {
    if hits.is_empty() {
        return "no sessions found".to_owned();
    }
    hits.iter()
        .map(format_hit)
        .collect::<Vec<_>>()
        .join("\n---\n")
}

fn format_hit(hit: &SessionHit) -> String {
    let handle = hit.session_handle.as_str();
    let title = if hit.title.is_empty() {
        "(untitled)".to_owned()
    } else {
        hit.title.clone()
    };
    let header = format!(
        "## {title}\nsession: {handle} · agent: {agent} · sender: {sender}\nupdated: {updated} · matched message #{idx}",
        agent = hit.agent,
        sender = hit.sender,
        updated = hit.updated_at,
        idx = hit.msg_idx,
    );
    let body = hit
        .window
        .iter()
        .map(format_item)
        .collect::<Vec<_>>()
        .join("\n");
    format!("{header}\n{body}")
}

fn format_item(item: &WindowItem) -> String {
    let role = role_label(item);
    let trunc = if item.truncated { " …" } else { "" };
    format!(
        "- [{role} #{idx}] {snippet}{trunc}",
        role = role,
        idx = item.msg_idx,
        snippet = item.snippet,
        trunc = trunc,
    )
}

fn role_label(item: &WindowItem) -> String {
    use wcore::model::Role;
    match item.role {
        Role::User => "user".to_owned(),
        Role::Assistant if item.tool_name.is_some() => format!(
            "tool-call:{}",
            item.tool_name.as_deref().unwrap_or_default()
        ),
        Role::Assistant => "assistant".to_owned(),
        Role::Tool => format!("tool:{}", item.tool_name.as_deref().unwrap_or_default()),
        Role::System => "system".to_owned(),
        _ => "other".to_owned(),
    }
}
