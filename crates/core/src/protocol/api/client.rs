//! Client trait — transport primitives plus typed provided methods.

use crate::protocol::message::{
    AgentInfo, AgentList, ClientMessage, ConversationHistory, ConversationInfo, ConversationList,
    CreateAgentMsg, DaemonStats, DeleteAgentMsg, DeleteConversationMsg, DeleteProviderMsg,
    ErrorMsg, GetAgentMsg, GetConversationHistoryMsg, GetStats, HubEvent, HubPackageInfo,
    HubPackageList, InstallPackageMsg, ListAgentsMsg, ListConversationsMsg, ListMcpsMsg,
    ListModelsMsg, ListPackagesMsg, ListProviderPresetsMsg, ListProvidersMsg, ListSkillsMsg,
    McpInfo, McpList, ModelInfo, ModelList, PackageInfo, PackageList, Ping, ProviderInfo,
    ProviderList, ProviderPresetInfo, ProviderPresetList, ResourceKind, SearchHubMsg, SendMsg,
    SendResponse, ServerMessage, ServiceLogOutput, ServiceLogsMsg, SetActiveModelMsg,
    SetEnabledMsg, SetLocalMcpsMsg, SetProviderMsg, SkillInfo, SkillList, StartServiceMsg,
    StopServiceMsg, StreamEvent, StreamMsg, UninstallPackageMsg, UpdateAgentMsg, client_message,
    hub_event, server_message, stream_event,
};
use anyhow::Result;
use futures_core::Stream;
use futures_util::StreamExt;

/// Client-side protocol interface.
///
/// Implementors provide two transport primitives — [`request`](Client::request)
/// for request-response and [`request_stream`](Client::request_stream) for
/// streaming operations. All typed methods are provided defaults that delegate
/// to these primitives.
pub trait Client: Send {
    /// Send a `ClientMessage` and receive a single `ServerMessage`.
    fn request(
        &mut self,
        msg: ClientMessage,
    ) -> impl std::future::Future<Output = Result<ServerMessage>> + Send;

    /// Send a `ClientMessage` and receive a stream of `ServerMessage`s.
    ///
    /// This is a raw transport primitive — the stream reads indefinitely.
    /// Callers must detect the terminal sentinel (e.g. `StreamEnd`)
    /// and stop consuming. The typed streaming methods handle this
    /// automatically.
    fn request_stream(
        &mut self,
        msg: ClientMessage,
    ) -> impl Stream<Item = Result<ServerMessage>> + Send + '_;

    /// Send a message to an agent and receive a complete response.
    fn send(
        &mut self,
        req: SendMsg,
    ) -> impl std::future::Future<Output = Result<SendResponse>> + Send {
        async move { SendResponse::try_from(self.request(req.into()).await?) }
    }

    /// Send a message to an agent and receive a streamed response.
    fn stream(
        &mut self,
        req: StreamMsg,
    ) -> impl Stream<Item = Result<stream_event::Event>> + Send + '_ {
        self.request_stream(req.into())
            .take_while(|r| {
                std::future::ready(!matches!(
                    r,
                    Ok(ServerMessage {
                        msg: Some(server_message::Msg::Stream(StreamEvent {
                            event: Some(stream_event::Event::End(_))
                        }))
                    })
                ))
            })
            .map(|r| r.and_then(stream_event::Event::try_from))
    }

    /// Ping the server (keepalive).
    fn ping(&mut self) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::Ping(Ping {})),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Get daemon stats including the active model name.
    fn get_stats(&mut self) -> impl std::future::Future<Output = Result<DaemonStats>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::GetStats(GetStats {})),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::Stats(stats)),
                } => Ok(stats),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// List all registered agents.
    fn list_agents(&mut self) -> impl std::future::Future<Output = Result<Vec<AgentInfo>>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::ListAgents(ListAgentsMsg {})),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::AgentList(AgentList { agents })),
                } => Ok(agents),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Get a single agent by name.
    fn get_agent(
        &mut self,
        name: String,
    ) -> impl std::future::Future<Output = Result<AgentInfo>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::GetAgent(GetAgentMsg { name })),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::AgentInfo(info)),
                } => Ok(info),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Create an agent from JSON config.
    fn create_agent(
        &mut self,
        name: String,
        config: String,
    ) -> impl std::future::Future<Output = Result<AgentInfo>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::CreateAgent(CreateAgentMsg {
                        name,
                        config,
                    })),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::AgentInfo(info)),
                } => Ok(info),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Update an agent from JSON config.
    fn update_agent(
        &mut self,
        name: String,
        config: String,
    ) -> impl std::future::Future<Output = Result<AgentInfo>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::UpdateAgent(UpdateAgentMsg {
                        name,
                        config,
                    })),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::AgentInfo(info)),
                } => Ok(info),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Delete an agent by name.
    fn delete_agent(
        &mut self,
        name: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::DeleteAgent(DeleteAgentMsg { name })),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// List all registered LLM providers.
    fn list_providers(
        &mut self,
    ) -> impl std::future::Future<Output = Result<Vec<ProviderInfo>>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::ListProviders(ListProvidersMsg {})),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::ProviderList(ProviderList { providers })),
                } => Ok(providers),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Install a hub package, streaming progress events.
    fn install_package(
        &mut self,
        package: String,
        branch: String,
        path: String,
        force: bool,
    ) -> impl Stream<Item = Result<hub_event::Event>> + Send + '_ {
        self.request_stream(ClientMessage {
            msg: Some(client_message::Msg::InstallPackage(InstallPackageMsg {
                package,
                branch,
                path,
                force,
            })),
        })
        .take_while(|r| {
            std::future::ready(!matches!(
                r,
                Ok(ServerMessage {
                    msg: Some(server_message::Msg::HubEvent(HubEvent {
                        event: Some(hub_event::Event::Done(d))
                    }))
                }) if d.error.is_empty()
            ))
        })
        .map(|r| r.and_then(hub_event::Event::try_from))
    }

    /// Uninstall a hub package, streaming progress events.
    fn uninstall_package(
        &mut self,
        package: String,
    ) -> impl Stream<Item = Result<hub_event::Event>> + Send + '_ {
        self.request_stream(ClientMessage {
            msg: Some(client_message::Msg::UninstallPackage(UninstallPackageMsg {
                package,
            })),
        })
        .take_while(|r| {
            std::future::ready(!matches!(
                r,
                Ok(ServerMessage {
                    msg: Some(server_message::Msg::HubEvent(HubEvent {
                        event: Some(hub_event::Event::Done(d))
                    }))
                }) if d.error.is_empty()
            ))
        })
        .map(|r| r.and_then(hub_event::Event::try_from))
    }

    /// List installed hub packages.
    fn list_packages(
        &mut self,
    ) -> impl std::future::Future<Output = Result<Vec<PackageInfo>>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::ListPackages(ListPackagesMsg {})),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::PackageList(PackageList { packages })),
                } => Ok(packages),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Search hub for available packages.
    fn search_hub(
        &mut self,
        query: String,
    ) -> impl std::future::Future<Output = Result<Vec<HubPackageInfo>>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::SearchHub(SearchHubMsg { query })),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::HubPackageList(HubPackageList { packages })),
                } => Ok(packages),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// List historical conversations from disk.
    fn list_conversations(
        &mut self,
        agent: String,
        sender: String,
    ) -> impl std::future::Future<Output = Result<Vec<ConversationInfo>>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::ListConversations(
                        ListConversationsMsg { agent, sender },
                    )),
                })
                .await?
            {
                ServerMessage {
                    msg:
                        Some(server_message::Msg::ConversationList(ConversationList { conversations })),
                } => Ok(conversations),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Load conversation history from a session file.
    fn get_conversation_history(
        &mut self,
        file_path: String,
    ) -> impl std::future::Future<Output = Result<ConversationHistory>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::GetConversationHistory(
                        GetConversationHistoryMsg { file_path },
                    )),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::ConversationHistory(history)),
                } => Ok(history),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Delete a conversation file from disk.
    fn delete_conversation(
        &mut self,
        file_path: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::DeleteConversation(
                        DeleteConversationMsg { file_path },
                    )),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// List all MCP server configs.
    fn list_mcps(&mut self) -> impl std::future::Future<Output = Result<Vec<McpInfo>>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::ListMcps(ListMcpsMsg {})),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::McpList(McpList { mcps })),
                } => Ok(mcps),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => anyhow::bail!("server error ({code}): {message}"),
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Replace all local MCPs in CrabTalk.toml.
    fn set_local_mcps(
        &mut self,
        mcps: Vec<McpInfo>,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::SetLocalMcps(SetLocalMcpsMsg { mcps })),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => anyhow::bail!("server error ({code}): {message}"),
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Create or update a provider.
    fn set_provider(
        &mut self,
        name: String,
        config: String,
    ) -> impl std::future::Future<Output = Result<ProviderInfo>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::SetProvider(SetProviderMsg {
                        name,
                        config,
                    })),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::ProviderList(ProviderList { providers })),
                } => providers
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("empty provider list in response")),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => anyhow::bail!("server error ({code}): {message}"),
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Delete a provider by name.
    fn delete_provider(
        &mut self,
        name: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::DeleteProvider(DeleteProviderMsg {
                        name,
                    })),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => anyhow::bail!("server error ({code}): {message}"),
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Set the active model.
    fn set_active_model(
        &mut self,
        model: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::SetActiveModel(SetActiveModelMsg {
                        model,
                    })),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => anyhow::bail!("server error ({code}): {message}"),
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// List provider presets.
    fn list_provider_presets(
        &mut self,
    ) -> impl std::future::Future<Output = Result<Vec<ProviderPresetInfo>>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::ListProviderPresets(
                        ListProviderPresetsMsg {},
                    )),
                })
                .await?
            {
                ServerMessage {
                    msg:
                        Some(server_message::Msg::ProviderPresetList(ProviderPresetList { presets })),
                } => Ok(presets),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => anyhow::bail!("server error ({code}): {message}"),
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Start a command service.
    fn start_service(
        &mut self,
        name: String,
        force: bool,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::StartService(StartServiceMsg {
                        name,
                        force,
                    })),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Stop a command service.
    fn stop_service(
        &mut self,
        name: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::StopService(StopServiceMsg { name })),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// List all available skills with enabled state.
    fn list_skills(&mut self) -> impl std::future::Future<Output = Result<Vec<SkillInfo>>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::ListSkills(ListSkillsMsg {})),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::SkillList(SkillList { skills })),
                } => Ok(skills),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// List all resolved models with provider and active state.
    fn list_models(&mut self) -> impl std::future::Future<Output = Result<Vec<ModelInfo>>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::ListModels(ListModelsMsg {})),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::ModelList(ModelList { models })),
                } => Ok(models),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Enable or disable a provider, MCP, or skill.
    fn set_enabled(
        &mut self,
        kind: ResourceKind,
        name: String,
        enabled: bool,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::SetEnabled(SetEnabledMsg {
                        kind: kind.into(),
                        name,
                        enabled,
                    })),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => anyhow::bail!("server error ({code}): {message}"),
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Get recent log lines for a service.
    fn service_logs(
        &mut self,
        name: String,
        lines: u32,
    ) -> impl std::future::Future<Output = Result<String>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::ServiceLogs(ServiceLogsMsg {
                        name,
                        lines,
                    })),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::ServiceLogOutput(ServiceLogOutput { content })),
                } => Ok(content),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }
}
