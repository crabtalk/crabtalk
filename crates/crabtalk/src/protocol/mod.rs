//! Server trait implementation — thin delegates to domain modules.

use crate::node::Node;
use anyhow::Result;
use crabllm_core::Provider;
use wcore::protocol::api::Server;
use wcore::protocol::message::*;

mod admin;
mod agent;
mod config;
mod conversation;
mod history;
mod plugin;

impl<P: Provider + 'static> Server for Node<P> {
    async fn send(&self, req: SendMsg) -> Result<SendResponse> {
        conversation::send(self, req).await
    }

    fn stream(
        &self,
        req: StreamMsg,
    ) -> impl futures_core::Stream<Item = Result<StreamEvent>> + Send {
        conversation::stream(self, req)
    }

    async fn compact_conversation(&self, agent: String, sender: String) -> Result<String> {
        conversation::compact(self, agent, sender).await
    }

    async fn ping(&self) -> Result<()> {
        admin::ping().await
    }

    async fn list_conversations_active(&self) -> Result<Vec<ActiveConversationInfo>> {
        conversation::list_active(self).await
    }

    async fn kill_conversation(&self, agent: String, sender: String) -> Result<bool> {
        conversation::kill(self, agent, sender).await
    }

    fn subscribe_events(&self) -> impl futures_core::Stream<Item = Result<AgentEventMsg>> + Send {
        admin::subscribe_events(self)
    }

    async fn reload(&self) -> Result<()> {
        admin::reload(self).await
    }

    async fn get_stats(&self) -> Result<DaemonStats> {
        admin::get_stats(self).await
    }

    async fn create_cron(&self, req: CreateCronMsg) -> Result<CronInfo> {
        admin::create_cron(self, req).await
    }

    async fn delete_cron(&self, id: u64) -> Result<bool> {
        admin::delete_cron(self, id).await
    }

    async fn list_crons(&self) -> Result<CronList> {
        admin::list_crons(self).await
    }

    async fn subscribe_event(&self, req: SubscribeEventMsg) -> Result<SubscriptionInfo> {
        admin::subscribe_event(self, req).await
    }

    async fn unsubscribe_event(&self, id: u64) -> Result<bool> {
        admin::unsubscribe_event(self, id).await
    }

    async fn list_subscriptions(&self) -> Result<SubscriptionList> {
        admin::list_subscriptions(self).await
    }

    async fn publish_event(&self, req: PublishEventMsg) -> Result<()> {
        admin::publish_event(self, req).await
    }

    async fn reply_to_ask(&self, agent: String, sender: String, content: String) -> Result<()> {
        conversation::reply_to_ask(self, agent, sender, content).await
    }

    async fn steer_session(&self, req: SteerSessionMsg) -> Result<()> {
        conversation::steer(self, req).await
    }

    async fn list_agents(&self) -> Result<Vec<AgentInfo>> {
        agent::list(self).await
    }

    async fn get_agent(&self, name: String) -> Result<AgentInfo> {
        agent::get(self, name).await
    }

    async fn create_agent(&self, req: CreateAgentMsg) -> Result<AgentInfo> {
        agent::create(self, req).await
    }

    async fn update_agent(&self, req: UpdateAgentMsg) -> Result<AgentInfo> {
        agent::update(self, req).await
    }

    async fn delete_agent(&self, name: String) -> Result<bool> {
        agent::delete(self, name).await
    }

    async fn list_providers(&self) -> Result<Vec<ProviderInfo>> {
        config::list_providers(self).await
    }

    fn install_plugin(
        &self,
        req: InstallPluginMsg,
    ) -> impl futures_core::Stream<Item = Result<PluginEvent>> + Send {
        plugin::install(self, req)
    }

    fn uninstall_plugin(
        &self,
        plugin: String,
    ) -> impl futures_core::Stream<Item = Result<PluginEvent>> + Send {
        plugin::uninstall(self, plugin)
    }

    async fn list_conversations(
        &self,
        agent: String,
        sender: String,
    ) -> Result<Vec<ConversationInfo>> {
        history::list_conversations(self, agent, sender).await
    }

    async fn get_conversation_history(&self, file_path: String) -> Result<ConversationHistory> {
        history::get_conversation_history(self, file_path).await
    }

    async fn delete_conversation(&self, file_path: String) -> Result<()> {
        history::delete_conversation(self, file_path).await
    }

    async fn list_mcps(&self) -> Result<Vec<McpInfo>> {
        config::list_mcps(self).await
    }

    async fn set_local_mcps(&self, mcps: Vec<McpInfo>) -> Result<()> {
        config::set_local_mcps(self, mcps).await
    }

    async fn set_provider(&self, name: String, config: String) -> Result<ProviderInfo> {
        config::set_provider(self, name, config).await
    }

    async fn delete_provider(&self, name: String) -> Result<()> {
        config::delete_provider(self, name).await
    }

    async fn set_active_model(&self, model: String) -> Result<()> {
        config::set_active_model(self, model).await
    }

    async fn list_provider_presets(&self) -> Result<Vec<ProviderPresetInfo>> {
        config::list_provider_presets().await
    }

    async fn list_skills(&self) -> Result<Vec<SkillInfo>> {
        config::list_skills(self).await
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        config::list_models(self).await
    }

    async fn set_enabled(&self, kind: ResourceKind, name: String, enabled: bool) -> Result<()> {
        config::set_enabled(self, kind, name, enabled).await
    }

    async fn list_plugins(&self) -> Result<Vec<PluginInfo>> {
        plugin::list(self).await
    }

    async fn search_plugins(&self, query: String) -> Result<Vec<PluginInfo>> {
        plugin::search(query).await
    }

    async fn start_service(&self, name: String, force: bool) -> Result<()> {
        admin::start_service(self, name, force).await
    }

    async fn stop_service(&self, name: String) -> Result<()> {
        admin::stop_service(name).await
    }

    async fn service_logs(&self, name: String, lines: u32) -> Result<String> {
        admin::service_logs(name, lines).await
    }
}
