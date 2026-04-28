//! Server trait — one async method per protocol operation.

use crate::protocol::message::{
    ActiveConversationInfo, ActiveConversationList, AgentEventMsg, AgentInfo, AgentList,
    ClientMessage, CompactResponse, ConversationHistory, ConversationInfo, ConversationList,
    CreateAgentMsg, DaemonStats, ErrorMsg, InstallPluginMsg, McpEventMsg, McpInfo, McpList,
    ModelInfo, ModelList, PluginEvent, PluginInfo, PluginList, PluginSearchList, Pong,
    PublishEventMsg, SendMsg, SendResponse, ServerMessage, ServiceLogOutput, SkillInfo, SkillList,
    SteerSessionMsg, StreamEvent, StreamMsg, SubscribeEventMsg, SubscriptionInfo, SubscriptionList,
    UpdateAgentMsg, UpsertMcpMsg, client_message, server_message,
};
use anyhow::Result;
use futures_core::Stream;
use futures_util::StreamExt;

/// Construct an error `ServerMessage`.
fn server_error(code: u32, message: String) -> ServerMessage {
    ServerMessage {
        msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
    }
}

/// Construct a pong `ServerMessage`.
fn server_pong() -> ServerMessage {
    ServerMessage {
        msg: Some(server_message::Msg::Pong(Pong {})),
    }
}

/// Convert a typed `Result` into a `ServerMessage`.
fn result_to_msg<T: Into<ServerMessage>>(result: Result<T>) -> ServerMessage {
    match result {
        Ok(resp) => resp.into(),
        Err(e) => server_error(500, e.to_string()),
    }
}

/// Server-side protocol handler.
///
/// Each method corresponds to one `ClientMessage` variant. Implementations
/// receive typed request structs and return typed responses — no enum matching
/// required. Streaming operations return `impl Stream`.
///
/// The provided [`dispatch`](Server::dispatch) method routes a raw
/// `ClientMessage` to the appropriate handler, returning a stream of
/// `ServerMessage`s.
pub trait Server: Sync {
    /// Handle `Send` — run agent and return complete response.
    fn send(&self, req: SendMsg) -> impl std::future::Future<Output = Result<SendResponse>> + Send;

    /// Handle `Stream` — run agent and stream response events.
    fn stream(&self, req: StreamMsg) -> impl Stream<Item = Result<StreamEvent>> + Send;

    /// Handle `Ping` — keepalive.
    fn ping(&self) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Handle `ListActiveConversations` — list active conversations.
    fn list_conversations_active(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<ActiveConversationInfo>>> + Send;

    /// Handle `Kill` — close a conversation by (agent, sender).
    fn kill_conversation(
        &self,
        agent: String,
        sender: String,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Handle `SubscribeEvents` — stream agent events.
    fn subscribe_events(&self) -> impl Stream<Item = Result<AgentEventMsg>> + Send;

    /// Handle `SubscribeMcpEvents` — stream MCP lifecycle events.
    fn subscribe_mcp_events(&self) -> impl Stream<Item = Result<McpEventMsg>> + Send;

    /// Handle `Reload` — hot-reload runtime from disk.
    fn reload(&self) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Handle `GetStats` — return daemon-level stats.
    fn get_stats(&self) -> impl std::future::Future<Output = Result<DaemonStats>> + Send;

    /// Handle `SubscribeEvent` — create an event bus subscription.
    fn subscribe_event(
        &self,
        req: SubscribeEventMsg,
    ) -> impl std::future::Future<Output = Result<SubscriptionInfo>> + Send;

    /// Handle `UnsubscribeEvent` — remove an event bus subscription.
    fn unsubscribe_event(&self, id: u64) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Handle `ListSubscriptions` — return all event bus subscriptions.
    fn list_subscriptions(
        &self,
    ) -> impl std::future::Future<Output = Result<SubscriptionList>> + Send;

    /// Handle `PublishEvent` — publish an event to the bus.
    fn publish_event(
        &self,
        req: PublishEventMsg,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Handle `Compact` — compact a conversation's history into a summary.
    fn compact_conversation(
        &self,
        agent: String,
        sender: String,
    ) -> impl std::future::Future<Output = Result<String>> + Send;

    /// Handle `ReplyToAsk` — deliver a user reply to a pending `ask_user` tool call.
    fn reply_to_ask(
        &self,
        agent: String,
        sender: String,
        content: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Handle `SteerSession` — inject a user message into an active stream.
    fn steer_session(
        &self,
        req: SteerSessionMsg,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Handle `ListAgents` — return all registered agents.
    fn list_agents(&self) -> impl std::future::Future<Output = Result<Vec<AgentInfo>>> + Send;

    /// Handle `GetAgent` — return a single agent by name.
    fn get_agent(
        &self,
        name: String,
    ) -> impl std::future::Future<Output = Result<AgentInfo>> + Send;

    /// Handle `CreateAgent` — create a new agent from JSON config.
    fn create_agent(
        &self,
        req: CreateAgentMsg,
    ) -> impl std::future::Future<Output = Result<AgentInfo>> + Send;

    /// Handle `UpdateAgent` — update an existing agent from JSON config.
    fn update_agent(
        &self,
        req: UpdateAgentMsg,
    ) -> impl std::future::Future<Output = Result<AgentInfo>> + Send;

    /// Handle `DeleteAgent` — remove an agent by name.
    fn delete_agent(&self, name: String) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Handle `RenameAgent` — rename an agent in place.
    fn rename_agent(
        &self,
        old_name: String,
        new_name: String,
    ) -> impl std::future::Future<Output = Result<AgentInfo>> + Send;

    /// Handle `InstallPlugin` — install a plugin, stream progress.
    fn install_plugin(
        &self,
        req: InstallPluginMsg,
    ) -> impl Stream<Item = Result<PluginEvent>> + Send;

    /// Handle `UninstallPlugin` — uninstall a plugin, stream progress.
    fn uninstall_plugin(&self, plugin: String) -> impl Stream<Item = Result<PluginEvent>> + Send;

    /// Handle `ListPlugins` — return all installed plugins.
    fn list_plugins(&self) -> impl std::future::Future<Output = Result<Vec<PluginInfo>>> + Send;

    /// Handle `SearchPlugins` — search registry for available plugins.
    fn search_plugins(
        &self,
        query: String,
    ) -> impl std::future::Future<Output = Result<Vec<PluginInfo>>> + Send;

    /// Handle `ListSkills` — return all available skills with enabled state.
    fn list_skills(&self) -> impl std::future::Future<Output = Result<Vec<SkillInfo>>> + Send;

    /// Handle `ListModels` — return all resolved models with provider and active state.
    fn list_models(&self) -> impl std::future::Future<Output = Result<Vec<ModelInfo>>> + Send;

    /// Handle `ListConversations` — return historical conversations from disk.
    fn list_conversations(
        &self,
        agent: String,
        sender: String,
    ) -> impl std::future::Future<Output = Result<Vec<ConversationInfo>>> + Send;

    /// Handle `GetConversationHistory` — load messages from a session file.
    fn get_conversation_history(
        &self,
        file_path: String,
    ) -> impl std::future::Future<Output = Result<ConversationHistory>> + Send;

    /// Handle `DeleteConversation` — delete a conversation file from disk.
    fn delete_conversation(
        &self,
        file_path: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Handle `ListMcps` — return all MCP server configs with source info.
    fn list_mcps(&self) -> impl std::future::Future<Output = Result<Vec<McpInfo>>> + Send;

    /// Handle `UpsertMcp` — create or replace an MCP server in Storage.
    fn upsert_mcp(
        &self,
        req: UpsertMcpMsg,
    ) -> impl std::future::Future<Output = Result<McpInfo>> + Send;

    /// Handle `DeleteMcp` — remove an MCP server from Storage.
    fn delete_mcp(&self, name: String) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Handle `SetActiveModel` — update the active model in config.toml.
    fn set_active_model(
        &self,
        model: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Handle `StartService` — install and start a command service.
    fn start_service(
        &self,
        name: String,
        force: bool,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Handle `StopService` — stop and uninstall a command service.
    fn stop_service(&self, name: String) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Handle `ServiceLogs` — return recent log lines for a service.
    fn service_logs(
        &self,
        name: String,
        lines: u32,
    ) -> impl std::future::Future<Output = Result<String>> + Send;

    /// Handle `Extension` — opaque bytes for downstream product protocols.
    ///
    /// Default: returns "not supported". Downstream products override this
    /// to handle their own message formats (local proto, JSON, bincode, etc.).
    fn dispatch_extension(
        &self,
        _payload: Vec<u8>,
    ) -> impl std::future::Future<Output = Result<Vec<u8>>> + Send {
        async { anyhow::bail!("extension not supported") }
    }

    /// Dispatch a `ClientMessage` to the appropriate handler method.
    ///
    /// Returns a stream of `ServerMessage`s. Request-response operations
    /// yield exactly one message; streaming operations yield many.
    fn dispatch(&self, msg: ClientMessage) -> impl Stream<Item = ServerMessage> + Send + '_ {
        async_stream::stream! {
            let Some(inner) = msg.msg else {
                yield server_error(400, "empty client message".to_string());
                return;
            };

            match inner {
                client_message::Msg::Send(send_msg) => {
                    yield result_to_msg(self.send(send_msg).await);
                }
                client_message::Msg::Stream(stream_msg) => {
                    let s = self.stream(stream_msg);
                    tokio::pin!(s);
                    while let Some(result) = s.next().await {
                        yield result_to_msg(result);
                    }
                }
                client_message::Msg::Ping(_) => {
                    yield match self.ping().await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::ListActiveConversations(_req) => {
                    yield match self.list_conversations_active().await {
                        Ok(conversations) => ServerMessage {
                            msg: Some(server_message::Msg::ActiveConversations(ActiveConversationList { conversations })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::Kill(kill_msg) => {
                    yield match self.kill_conversation(kill_msg.agent.clone(), kill_msg.sender.clone()).await {
                        Ok(true) => server_pong(),
                        Ok(false) => server_error(
                            404,
                            format!("conversation not found for agent='{}' sender='{}'", kill_msg.agent, kill_msg.sender),
                        ),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::GetConfig(_) => {
                    yield server_error(410, "GetConfig is deprecated — use GetStats and granular APIs".into());
                }
                client_message::Msg::SubscribeEvents(_) => {
                    let s = self.subscribe_events();
                    tokio::pin!(s);
                    while let Some(result) = s.next().await {
                        yield result_to_msg(result);
                    }
                }
                client_message::Msg::SubscribeMcpEvents(_) => {
                    let s = self.subscribe_mcp_events();
                    tokio::pin!(s);
                    while let Some(result) = s.next().await {
                        yield result_to_msg(result);
                    }
                }
                client_message::Msg::Reload(_) => {
                    yield match self.reload().await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::ReplyToAsk(msg) => {
                    yield match self.reply_to_ask(msg.agent, msg.sender, msg.content).await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(404, e.to_string()),
                    };
                }
                client_message::Msg::SteerSession(req) => {
                    yield match self.steer_session(req).await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::GetStats(_) => {
                    yield match self.get_stats().await {
                        Ok(stats) => ServerMessage {
                            msg: Some(server_message::Msg::Stats(stats)),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::SubscribeEvent(req) => {
                    yield match self.subscribe_event(req).await {
                        Ok(info) => ServerMessage {
                            msg: Some(server_message::Msg::SubscriptionInfo(info)),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::UnsubscribeEvent(req) => {
                    yield match self.unsubscribe_event(req.id).await {
                        Ok(true) => server_pong(),
                        Ok(false) => server_error(404, format!("subscription {} not found", req.id)),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::ListSubscriptions(_) => {
                    yield match self.list_subscriptions().await {
                        Ok(list) => ServerMessage {
                            msg: Some(server_message::Msg::SubscriptionList(list)),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::PublishEvent(req) => {
                    yield match self.publish_event(req).await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::Compact(req) => {
                    yield match self.compact_conversation(req.agent, req.sender).await {
                        Ok(summary) => ServerMessage {
                            msg: Some(server_message::Msg::Compact(CompactResponse { summary })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::ListAgents(_) => {
                    yield match self.list_agents().await {
                        Ok(agents) => ServerMessage {
                            msg: Some(server_message::Msg::AgentList(AgentList { agents })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::GetAgent(req) => {
                    yield match self.get_agent(req.name).await {
                        Ok(info) => ServerMessage {
                            msg: Some(server_message::Msg::AgentInfo(info)),
                        },
                        Err(e) => server_error(404, e.to_string()),
                    };
                }
                client_message::Msg::CreateAgent(req) => {
                    yield match self.create_agent(req).await {
                        Ok(info) => ServerMessage {
                            msg: Some(server_message::Msg::AgentInfo(info)),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::UpdateAgent(req) => {
                    yield match self.update_agent(req).await {
                        Ok(info) => ServerMessage {
                            msg: Some(server_message::Msg::AgentInfo(info)),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::DeleteAgent(req) => {
                    yield match self.delete_agent(req.name.clone()).await {
                        Ok(true) => server_pong(),
                        Ok(false) => server_error(
                            404,
                            format!("agent '{}' not found in local manifest", req.name),
                        ),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::RenameAgent(req) => {
                    yield match self.rename_agent(req.old_name, req.new_name).await {
                        Ok(info) => ServerMessage {
                            msg: Some(server_message::Msg::AgentInfo(info)),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::InstallPlugin(req) => {
                    let s = self.install_plugin(req);
                    tokio::pin!(s);
                    while let Some(result) = s.next().await {
                        yield result_to_msg(result);
                    }
                }
                client_message::Msg::UninstallPlugin(req) => {
                    let s = self.uninstall_plugin(req.plugin);
                    tokio::pin!(s);
                    while let Some(result) = s.next().await {
                        yield result_to_msg(result);
                    }
                }
                client_message::Msg::ListPlugins(_) => {
                    yield match self.list_plugins().await {
                        Ok(plugins) => ServerMessage {
                            msg: Some(server_message::Msg::PluginList(PluginList {
                                plugins,
                            })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::SearchPlugins(req) => {
                    yield match self.search_plugins(req.query).await {
                        Ok(plugins) => ServerMessage {
                            msg: Some(server_message::Msg::PluginSearchList(PluginSearchList {
                                plugins,
                            })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::StartService(req) => {
                    yield match self.start_service(req.name, req.force).await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::StopService(req) => {
                    yield match self.stop_service(req.name).await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::ServiceLogs(req) => {
                    yield match self.service_logs(req.name, req.lines).await {
                        Ok(content) => ServerMessage {
                            msg: Some(server_message::Msg::ServiceLogOutput(
                                ServiceLogOutput { content },
                            )),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::ListSkills(_) => {
                    yield match self.list_skills().await {
                        Ok(skills) => ServerMessage {
                            msg: Some(server_message::Msg::SkillList(SkillList { skills })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::ListModels(_) => {
                    yield match self.list_models().await {
                        Ok(models) => ServerMessage {
                            msg: Some(server_message::Msg::ModelList(ModelList { models })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::ListConversations(req) => {
                    yield match self.list_conversations(req.agent, req.sender).await {
                        Ok(conversations) => ServerMessage {
                            msg: Some(server_message::Msg::ConversationList(ConversationList {
                                conversations,
                            })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::GetConversationHistory(req) => {
                    yield result_to_msg(self.get_conversation_history(req.file_path).await);
                }
                client_message::Msg::DeleteConversation(req) => {
                    yield match self.delete_conversation(req.file_path).await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::ListMcps(_) => {
                    yield match self.list_mcps().await {
                        Ok(mcps) => ServerMessage {
                            msg: Some(server_message::Msg::McpList(McpList { mcps })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::UpsertMcp(req) => {
                    yield match self.upsert_mcp(req).await {
                        Ok(info) => ServerMessage {
                            msg: Some(server_message::Msg::McpInfo(info)),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::DeleteMcp(req) => {
                    yield match self.delete_mcp(req.name.clone()).await {
                        Ok(true) => server_pong(),
                        Ok(false) => server_error(404, format!("mcp '{}' not found", req.name)),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::SetActiveModel(req) => {
                    yield match self.set_active_model(req.model).await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::Extension(payload) => {
                    yield match self.dispatch_extension(payload).await {
                        Ok(response) => ServerMessage {
                            msg: Some(server_message::Msg::Extension(response)),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
            }
        }
    }
}
