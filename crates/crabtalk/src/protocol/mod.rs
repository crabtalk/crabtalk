//! Server trait implementation — thin delegates to domain modules.

use crate::{llm::Provider, system::CrabTalk};
use anyhow::Result;
use proto::{server::Server, *};
use runtime::Sessions;
use serde_json::Value;
use std::sync::Arc;
use store::{AgentId, SessionHandle, interface::Backend};

mod admin;
mod config;
mod session;

impl<P: Provider + 'static, S: Backend> Server for CrabTalk<P, S> {
    async fn send(&self, req: SendMsg) -> Result<SendResponse> {
        self.send(req).await
    }

    fn stream(
        &self,
        req: StreamMsg,
    ) -> impl futures_core::Stream<Item = Result<StreamEvent>> + Send {
        self.stream(req)
    }

    async fn compact_conversation(&self, session_handle: String, prompt: String) -> Result<String> {
        let rt = self.runtime.read().await.clone();
        let handle = SessionHandle::new(session_handle.as_str());
        let (id, session) = self
            .sessions
            .find_by_handle(&handle)
            .ok_or_else(|| anyhow::anyhow!("session not found for handle='{session_handle}'"))?;
        let cancel = self.sessions.begin_cancel(id);
        let _cancel_guard = CancelGuard::new(self.sessions.clone(), id);
        rt.compact(&session, &prompt, cancel)
            .await
            .ok_or_else(|| anyhow::anyhow!("compact failed for handle='{session_handle}'"))
    }

    async fn ping(&self) -> Result<()> {
        Ok(())
    }

    async fn list_conversations_active(&self) -> Result<Vec<ActiveConversationInfo>> {
        let rt = self.runtime.read().await.clone();
        Ok(self.sessions.list_active(&rt).await)
    }

    async fn kill_conversation(&self, session_handle: String) -> Result<bool> {
        self.kill_conversation(&session_handle).await
    }

    fn subscribe_events(&self) -> impl futures_core::Stream<Item = Result<AgentEventMsg>> + Send {
        self.subscribe_events()
    }

    fn subscribe_mcp_events(&self) -> impl futures_core::Stream<Item = Result<McpEventMsg>> + Send {
        self.subscribe_mcp_events()
    }

    async fn get_stats(&self) -> Result<Stats> {
        self.get_stats().await
    }

    async fn subscribe_event(&self, req: SubscribeEventMsg) -> Result<SubscriptionInfo> {
        self.subscribe_event(req).await
    }

    async fn unsubscribe_event(&self, id: u64) -> Result<bool> {
        Ok(self.unsubscribe_event(id))
    }

    async fn list_subscriptions(&self) -> Result<SubscriptionList> {
        Ok(self.list_subscriptions())
    }

    async fn publish_event(&self, req: PublishEventMsg) -> Result<()> {
        self.publish_event(&req.source, &req.payload);
        Ok(())
    }

    async fn cancel_stream(&self, req: CancelStreamMsg) -> Result<()> {
        let handle = SessionHandle::new(req.session_handle.as_str());
        let (id, _) = self.sessions.find_by_handle(&handle).ok_or_else(|| {
            anyhow::anyhow!("session not found for handle='{}'", req.session_handle)
        })?;
        self.sessions.cancel(id)
    }

    async fn list_agents(&self) -> Result<Vec<AgentInfo>> {
        let rt = self.runtime.read().await.clone();
        Ok(rt.agents().await.into_iter().map(Into::into).collect())
    }

    async fn get_agent(&self, name: String) -> Result<AgentInfo> {
        let rt = self.runtime.read().await.clone();
        let config = rt
            .storage()
            .load_agent_by_name(&name)
            .await?
            .ok_or_else(|| anyhow::anyhow!("agent '{name}' not found"))?;
        Ok(config.into())
    }

    async fn create_agent(&self, req: CreateAgentMsg) -> Result<AgentInfo> {
        let mut config: store::AgentConfig = serde_json::from_str(&req.config)
            .map_err(|e| anyhow::anyhow!("invalid AgentConfig JSON: {e}"))?;
        config.name = req.name;
        let rt = self.runtime.read().await.clone();
        let registered = rt.create_agent(config).await?;
        Ok(registered.into())
    }

    async fn update_agent(&self, req: UpdateAgentMsg) -> Result<AgentInfo> {
        let id = parse_agent(&req.agent)?;
        let patch: Value = serde_json::from_str(&req.config)
            .map_err(|e| anyhow::anyhow!("invalid AgentConfig JSON: {e}"))?;
        let rt = self.runtime.read().await.clone();
        let stored = rt
            .storage()
            .load_agent(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("agent '{id}' not found"))?;
        // The name is not the patch's to change — `rename_agent` is.
        let name = stored.name.clone();
        let mut merged = serde_json::to_value(stored)?;
        self::merge(&mut merged, patch);
        let mut config: store::AgentConfig = serde_json::from_value(merged)
            .map_err(|e| anyhow::anyhow!("invalid AgentConfig JSON: {e}"))?;
        config.name = name;
        let registered = rt.update_agent(&id, config).await?;
        Ok(registered.into())
    }

    async fn delete_agent(&self, agent: String) -> Result<bool> {
        let rt = self.runtime.read().await.clone();
        rt.purge_agent(&parse_agent(&agent)?).await
    }

    async fn rename_agent(&self, agent: String, new_name: String) -> Result<AgentInfo> {
        let rt = self.runtime.read().await.clone();
        let registered = rt.rename_agent(&parse_agent(&agent)?, &new_name).await?;
        Ok(registered.into())
    }

    async fn list_conversations(
        &self,
        agent: String,
        sender: String,
    ) -> Result<Vec<ConversationInfo>> {
        let rt = self.runtime.read().await.clone();
        Ok(rt
            .list_conversations(parse_agent_filter(&agent)?, &sender)
            .await
            .into_iter()
            .map(|mut c| {
                c.date = self::format_date_label(&c.date);
                c
            })
            .collect())
    }

    async fn get_conversation_history(&self, file_path: String) -> Result<ConversationHistory> {
        let rt = self.runtime.read().await.clone();
        rt.load_conversation_history(&file_path).await
    }

    async fn delete_conversation(&self, file_path: String) -> Result<()> {
        let rt = self.runtime.read().await.clone();
        rt.delete_conversation(&file_path).await
    }

    async fn list_mcps(&self, req: ListMcpsMsg) -> Result<Vec<McpInfo>> {
        self.list_mcps(parse_agent_filter(&req.agent)?).await
    }

    async fn upsert_mcp(&self, req: UpsertMcpMsg) -> Result<McpInfo> {
        self.upsert_mcp(parse_agent(&req.agent)?, req.config).await
    }

    async fn delete_mcp(&self, req: DeleteMcpMsg) -> Result<bool> {
        self.delete_mcp(parse_agent(&req.agent)?, req.name).await
    }

    async fn reconnect_mcp(&self, req: ReconnectMcpMsg) -> Result<McpInfo> {
        self.reconnect_mcp(parse_agent(&req.agent)?, req.name).await
    }

    async fn set_active_model(&self, model: String) -> Result<()> {
        self.set_active_model(model).await
    }

    async fn list_skills(&self) -> Result<Vec<SkillInfo>> {
        Ok(self.list_skills().await)
    }

    async fn get_skill(&self, name: String) -> Result<SkillBody> {
        self.get_skill(name).await
    }

    async fn search_sessions(&self, req: SearchSessionsMsg) -> Result<Vec<SessionHit>> {
        self.search_sessions(req).await
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let rt = self.runtime.read().await.clone();
        Ok(rt.list_models().await)
    }
}

/// RAII guard that clears a session's cancellation token on every exit
/// path out of the cancellable operation it guards — stream end, early
/// return, compact's completion, or the caller's future being dropped.
pub(crate) struct CancelGuard {
    sessions: Arc<Sessions>,
    session_id: u64,
}

impl CancelGuard {
    pub(crate) fn new(sessions: Arc<Sessions>, session_id: u64) -> Self {
        Self {
            sessions,
            session_id,
        }
    }
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        self.sessions.end_cancel(self.session_id);
    }
}

/// Parse an agent ULID off the wire. There is no name fallback: a
/// caller with a name resolves it through `GetAgent` first.
pub(crate) fn parse_agent(raw: &str) -> Result<AgentId> {
    raw.parse()
        .map_err(|e| anyhow::anyhow!("invalid agent id '{raw}': {e}"))
}

/// The same, for a filter where empty means unrestricted.
fn parse_agent_filter(raw: &str) -> Result<Option<AgentId>> {
    if raw.is_empty() {
        return Ok(None);
    }
    parse_agent(raw).map(Some)
}

/// Render an RFC3339 `created_at` string as a human-friendly relative date —
/// "Today" / "Yesterday" / `YYYY-MM-DD`. Returns empty string if parsing fails.
fn format_date_label(created_at: &str) -> String {
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

/// Apply an RFC 7386 merge patch: objects merge key by key, anything else
/// replaces, and an explicit `null` removes a key so the field falls back to
/// its default. A key the patch never mentions keeps the stored value.
fn merge(base: &mut Value, patch: Value) {
    match (base, patch) {
        (Value::Object(base), Value::Object(patch)) => {
            for (key, value) in patch {
                if value.is_null() {
                    base.remove(&key);
                } else {
                    merge(base.entry(key).or_insert(Value::Null), value);
                }
            }
        }
        (base, patch) => *base = patch,
    }
}
