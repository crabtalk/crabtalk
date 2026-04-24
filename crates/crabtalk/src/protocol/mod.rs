//! Server trait implementation — thin delegates to domain modules.

use crate::daemon::Daemon;
use anyhow::Result;
use crabllm_core::Provider;
use wcore::protocol::api::Server;
use wcore::protocol::message::*;

mod admin;
mod config;
mod conversation;
mod plugin;

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

impl<P: Provider + 'static> Server for Daemon<P> {
    async fn send(&self, req: SendMsg) -> Result<SendResponse> {
        self.send(req).await
    }

    fn stream(
        &self,
        req: StreamMsg,
    ) -> impl futures_core::Stream<Item = Result<StreamEvent>> + Send {
        self.stream(req)
    }

    async fn compact_conversation(&self, agent: String, sender: String) -> Result<String> {
        let rt = self.runtime.read().await.clone();
        rt.compact_conversation(&agent, &sender).await
    }

    async fn ping(&self) -> Result<()> {
        Ok(())
    }

    async fn list_conversations_active(&self) -> Result<Vec<ActiveConversationInfo>> {
        let rt = self.runtime.read().await.clone();
        Ok(rt.list_active().await)
    }

    async fn kill_conversation(&self, agent: String, sender: String) -> Result<bool> {
        self.kill_conversation(&agent, &sender).await
    }

    fn subscribe_events(&self) -> impl futures_core::Stream<Item = Result<AgentEventMsg>> + Send {
        self.subscribe_events()
    }

    async fn reload(&self) -> Result<()> {
        self.reload().await
    }

    async fn get_stats(&self) -> Result<DaemonStats> {
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

    async fn reply_to_ask(&self, agent: String, sender: String, content: String) -> Result<()> {
        self.reply_to_ask(&agent, &sender, content).await
    }

    async fn steer_session(&self, req: SteerSessionMsg) -> Result<()> {
        let rt = self.runtime.read().await.clone();
        let sender = if req.sender.is_empty() {
            "user".to_owned()
        } else {
            req.sender
        };
        rt.steer_conversation(&req.agent, &sender, req.content)
            .await
    }

    async fn list_agents(&self) -> Result<Vec<AgentInfo>> {
        let rt = self.runtime.read().await.clone();
        Ok(rt.agents().iter().map(AgentInfo::from).collect())
    }

    async fn get_agent(&self, name: String) -> Result<AgentInfo> {
        let rt = self.runtime.read().await.clone();
        let config = rt
            .agent(&name)
            .ok_or_else(|| anyhow::anyhow!("agent '{name}' not found"))?;
        Ok(AgentInfo::from(&config))
    }

    async fn create_agent(&self, req: CreateAgentMsg) -> Result<AgentInfo> {
        let mut config: wcore::AgentConfig = serde_json::from_str(&req.config)
            .map_err(|e| anyhow::anyhow!("invalid AgentConfig JSON: {e}"))?;
        config.name = req.name;
        let rt = self.runtime.read().await.clone();
        let registered = rt.create_agent(config, &req.prompt)?;
        Ok(AgentInfo::from(&registered))
    }

    async fn update_agent(&self, req: UpdateAgentMsg) -> Result<AgentInfo> {
        let mut config: wcore::AgentConfig = serde_json::from_str(&req.config)
            .map_err(|e| anyhow::anyhow!("invalid AgentConfig JSON: {e}"))?;
        config.name = req.name;
        let rt = self.runtime.read().await.clone();
        let registered = rt.update_agent(config, &req.prompt)?;
        Ok(AgentInfo::from(&registered))
    }

    async fn delete_agent(&self, name: String) -> Result<bool> {
        let rt = self.runtime.read().await.clone();
        rt.purge_agent(&name)
    }

    async fn rename_agent(&self, old_name: String, new_name: String) -> Result<AgentInfo> {
        let rt = self.runtime.read().await.clone();
        let registered = rt.rename_agent(&old_name, &new_name)?;
        Ok(AgentInfo::from(&registered))
    }

    fn install_plugin(
        &self,
        req: InstallPluginMsg,
    ) -> impl futures_core::Stream<Item = Result<PluginEvent>> + Send {
        self.install_plugin(req)
    }

    fn uninstall_plugin(
        &self,
        plugin: String,
    ) -> impl futures_core::Stream<Item = Result<PluginEvent>> + Send {
        self.uninstall_plugin(plugin)
    }

    async fn list_conversations(
        &self,
        agent: String,
        sender: String,
    ) -> Result<Vec<ConversationInfo>> {
        let rt = self.runtime.read().await.clone();
        Ok(rt
            .list_conversations(&agent, &sender)
            .into_iter()
            .map(|mut c| {
                c.date = format_date_label(&c.date);
                c
            })
            .collect())
    }

    async fn get_conversation_history(&self, file_path: String) -> Result<ConversationHistory> {
        let rt = self.runtime.read().await.clone();
        rt.load_conversation_history(&file_path)
    }

    async fn delete_conversation(&self, file_path: String) -> Result<()> {
        let rt = self.runtime.read().await.clone();
        rt.delete_conversation(&file_path)
    }

    async fn list_mcps(&self) -> Result<Vec<McpInfo>> {
        self.list_mcps().await
    }

    async fn upsert_mcp(&self, req: UpsertMcpMsg) -> Result<McpInfo> {
        self.upsert_mcp(req.config).await
    }

    async fn delete_mcp(&self, name: String) -> Result<bool> {
        self.delete_mcp(&name).await
    }

    async fn set_active_model(&self, model: String) -> Result<()> {
        self.set_active_model(model).await
    }

    async fn list_skills(&self) -> Result<Vec<SkillInfo>> {
        Ok(self.list_skills())
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let rt = self.runtime.read().await.clone();
        Ok(rt.list_models())
    }

    async fn list_plugins(&self) -> Result<Vec<PluginInfo>> {
        Ok(self.list_plugins())
    }

    async fn search_plugins(&self, query: String) -> Result<Vec<PluginInfo>> {
        plugin::search(&query).await
    }

    async fn start_service(&self, name: String, force: bool) -> Result<()> {
        self.start_service(name, force).await
    }

    async fn stop_service(&self, name: String) -> Result<()> {
        admin::stop_service(&name).await
    }

    async fn service_logs(&self, name: String, lines: u32) -> Result<String> {
        admin::service_logs(&name, lines).await
    }
}
