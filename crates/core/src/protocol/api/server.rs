//! Server trait — one async method per protocol operation.

use crate::protocol::message::{
    AgentEventMsg, AgentInfo, AgentList, ClientMessage, CompactResponse, ConversationHistory,
    ConversationInfo, ConversationList, CreateAgentMsg, CreateCronMsg, CronInfo, CronList,
    DaemonStats, ErrorMsg, HubEvent, HubPackageInfo, HubPackageList, InstallPackageMsg, McpInfo,
    McpList, ModelInfo, ModelList, PackageInfo, PackageList, Pong, ProviderInfo, ProviderList,
    ProviderPresetInfo, ProviderPresetList, ResourceKind, SendMsg, SendResponse, ServerMessage,
    ServiceLogOutput, SessionInfo, SessionList, SkillInfo, SkillList, StreamEvent, StreamMsg,
    UpdateAgentMsg, client_message, server_message,
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

    /// Handle `Sessions` — list active sessions.
    fn list_sessions(&self) -> impl std::future::Future<Output = Result<Vec<SessionInfo>>> + Send;

    /// Handle `Kill` — close a session by ID.
    fn kill_session(&self, session: u64) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Handle `SubscribeEvents` — stream agent events.
    fn subscribe_events(&self) -> impl Stream<Item = Result<AgentEventMsg>> + Send;

    /// Handle `Reload` — hot-reload runtime from disk.
    fn reload(&self) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Handle `GetStats` — return daemon-level stats.
    fn get_stats(&self) -> impl std::future::Future<Output = Result<DaemonStats>> + Send;

    /// Handle `CreateCron` — create a new cron entry and start its timer.
    fn create_cron(
        &self,
        req: CreateCronMsg,
    ) -> impl std::future::Future<Output = Result<CronInfo>> + Send;

    /// Handle `DeleteCron` — remove a cron entry and stop its timer.
    fn delete_cron(&self, id: u64) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Handle `ListCrons` — return all cron entries.
    fn list_crons(&self) -> impl std::future::Future<Output = Result<CronList>> + Send;

    /// Handle `Compact` — compact a session's history into a summary.
    fn compact_session(
        &self,
        session: u64,
    ) -> impl std::future::Future<Output = Result<String>> + Send;

    /// Handle `ReplyToAsk` — deliver a user reply to a pending `ask_user` tool call.
    fn reply_to_ask(
        &self,
        session: u64,
        content: String,
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

    /// Handle `ListProviders` — return all registered LLM providers.
    fn list_providers(&self)
    -> impl std::future::Future<Output = Result<Vec<ProviderInfo>>> + Send;

    /// Handle `InstallPackage` — install a hub package, stream progress.
    fn install_package(
        &self,
        req: InstallPackageMsg,
    ) -> impl Stream<Item = Result<HubEvent>> + Send;

    /// Handle `UninstallPackage` — uninstall a hub package, stream progress.
    fn uninstall_package(&self, package: String) -> impl Stream<Item = Result<HubEvent>> + Send;

    /// Handle `ListPackages` — return all installed hub packages.
    fn list_packages(&self) -> impl std::future::Future<Output = Result<Vec<PackageInfo>>> + Send;

    /// Handle `SearchHub` — search hub for available packages.
    fn search_hub(
        &self,
        query: String,
    ) -> impl std::future::Future<Output = Result<Vec<HubPackageInfo>>> + Send;

    /// Handle `ListSkills` — return all available skills with enabled state.
    fn list_skills(&self) -> impl std::future::Future<Output = Result<Vec<SkillInfo>>> + Send;

    /// Handle `ListModels` — return all resolved models with provider and active state.
    fn list_models(&self) -> impl std::future::Future<Output = Result<Vec<ModelInfo>>> + Send;

    /// Handle `SetEnabled` — enable or disable a provider, MCP, or skill.
    fn set_enabled(
        &self,
        kind: ResourceKind,
        name: String,
        enabled: bool,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

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

    /// Handle `SetLocalMcps` — replace local MCPs in CrabTalk.toml.
    fn set_local_mcps(
        &self,
        mcps: Vec<McpInfo>,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Handle `SetProvider` — create or update a provider in config.toml.
    fn set_provider(
        &self,
        name: String,
        config: String,
    ) -> impl std::future::Future<Output = Result<ProviderInfo>> + Send;

    /// Handle `DeleteProvider` — remove a provider from config.toml.
    fn delete_provider(&self, name: String)
    -> impl std::future::Future<Output = Result<()>> + Send;

    /// Handle `SetActiveModel` — update the active model in config.toml.
    fn set_active_model(
        &self,
        model: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Handle `ListProviderPresets` — return provider preset templates.
    fn list_provider_presets(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<ProviderPresetInfo>>> + Send;

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
                client_message::Msg::Sessions(_) => {
                    yield match self.list_sessions().await {
                        Ok(sessions) => ServerMessage {
                            msg: Some(server_message::Msg::Sessions(SessionList { sessions })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::Kill(kill_msg) => {
                    yield match self.kill_session(kill_msg.session).await {
                        Ok(true) => server_pong(),
                        Ok(false) => server_error(
                            404,
                            format!("session {} not found", kill_msg.session),
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
                client_message::Msg::Reload(_) => {
                    yield match self.reload().await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::ReplyToAsk(msg) => {
                    yield match self.reply_to_ask(msg.session, msg.content).await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(404, e.to_string()),
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
                client_message::Msg::CreateCron(req) => {
                    yield match self.create_cron(req).await {
                        Ok(info) => ServerMessage {
                            msg: Some(server_message::Msg::CronInfo(info)),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::DeleteCron(req) => {
                    yield match self.delete_cron(req.id).await {
                        Ok(true) => server_pong(),
                        Ok(false) => server_error(404, format!("cron {} not found", req.id)),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::ListCrons(_) => {
                    yield match self.list_crons().await {
                        Ok(list) => ServerMessage {
                            msg: Some(server_message::Msg::CronList(list)),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::Compact(req) => {
                    yield match self.compact_session(req.session).await {
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
                client_message::Msg::ListProviders(_) => {
                    yield match self.list_providers().await {
                        Ok(providers) => ServerMessage {
                            msg: Some(server_message::Msg::ProviderList(ProviderList {
                                providers,
                            })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::InstallPackage(req) => {
                    let s = self.install_package(req);
                    tokio::pin!(s);
                    while let Some(result) = s.next().await {
                        yield result_to_msg(result);
                    }
                }
                client_message::Msg::UninstallPackage(req) => {
                    let s = self.uninstall_package(req.package);
                    tokio::pin!(s);
                    while let Some(result) = s.next().await {
                        yield result_to_msg(result);
                    }
                }
                client_message::Msg::ListPackages(_) => {
                    yield match self.list_packages().await {
                        Ok(packages) => ServerMessage {
                            msg: Some(server_message::Msg::PackageList(PackageList {
                                packages,
                            })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::SearchHub(req) => {
                    yield match self.search_hub(req.query).await {
                        Ok(packages) => ServerMessage {
                            msg: Some(server_message::Msg::HubPackageList(HubPackageList {
                                packages,
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
                client_message::Msg::SetEnabled(req) => {
                    let kind = ResourceKind::try_from(req.kind)
                        .unwrap_or(ResourceKind::Unknown);
                    yield match self.set_enabled(kind, req.name, req.enabled).await {
                        Ok(()) => server_pong(),
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
                client_message::Msg::SetLocalMcps(req) => {
                    yield match self.set_local_mcps(req.mcps).await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::SetProvider(req) => {
                    yield match self.set_provider(req.name, req.config).await {
                        Ok(info) => ServerMessage {
                            msg: Some(server_message::Msg::ProviderList(ProviderList {
                                providers: vec![info],
                            })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::DeleteProvider(req) => {
                    yield match self.delete_provider(req.name.clone()).await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::SetActiveModel(req) => {
                    yield match self.set_active_model(req.model).await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::ListProviderPresets(_) => {
                    yield match self.list_provider_presets().await {
                        Ok(presets) => ServerMessage {
                            msg: Some(server_message::Msg::ProviderPresetList(
                                ProviderPresetList { presets },
                            )),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
            }
        }
    }
}
