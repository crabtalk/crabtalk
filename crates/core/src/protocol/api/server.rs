//! Server trait — one async method per protocol operation.

use crate::protocol::message::{
    AgentEventMsg, AgentInfo, AgentList, ClientMessage, CompactResponse, ConfigMsg,
    ConversationInfo, ConversationList, CreateAgentMsg, CreateCronMsg, CronInfo, CronList,
    DaemonStats, ErrorMsg, HubEvent, InstallPackageMsg, PackageInfo, PackageList, Pong,
    ProviderInfo, ProviderList, SendMsg, SendResponse, ServerMessage, ServiceLogOutput,
    SessionInfo, SessionList, SkillList, StreamEvent, StreamMsg, UpdateAgentMsg, client_message,
    server_message,
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

    /// Handle `GetConfig` — return the full daemon config as JSON.
    fn get_config(&self) -> impl std::future::Future<Output = Result<String>> + Send;

    /// Handle `SetConfig` — replace the daemon config from JSON.
    fn set_config(&self, config: String) -> impl std::future::Future<Output = Result<()>> + Send;

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

    /// Handle `ListSkills` — return all available skill names.
    fn list_skills(&self) -> impl std::future::Future<Output = Result<Vec<String>>> + Send;

    /// Handle `ListConversations` — return historical conversations from disk.
    fn list_conversations(
        &self,
        agent: String,
        sender: String,
    ) -> impl std::future::Future<Output = Result<Vec<ConversationInfo>>> + Send;

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
                    yield match self.get_config().await {
                        Ok(config) => ServerMessage {
                            msg: Some(server_message::Msg::Config(ConfigMsg { config })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::SetConfig(set_config_msg) => {
                    yield match self.set_config(set_config_msg.config).await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(500, e.to_string()),
                    };
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
                        Ok(names) => ServerMessage {
                            msg: Some(server_message::Msg::SkillList(SkillList { names })),
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
            }
        }
    }
}
