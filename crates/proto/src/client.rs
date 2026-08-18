//! Client trait — transport primitives plus typed provided methods.
#![cfg(feature = "client")]

use crate::{client_message, server_message, stream_event};
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
        msg: crate::ClientMessage,
    ) -> impl std::future::Future<Output = Result<crate::ServerMessage>> + Send;

    /// Send a `ClientMessage` and receive a stream of `ServerMessage`s.
    ///
    /// This is a raw transport primitive — the stream reads indefinitely.
    /// Callers must detect the terminal sentinel (e.g. `StreamEnd`)
    /// and stop consuming. The typed streaming methods handle this
    /// automatically.
    fn request_stream(
        &mut self,
        msg: crate::ClientMessage,
    ) -> impl Stream<Item = Result<crate::ServerMessage>> + Send + '_;

    /// Send a message to an agent and receive a complete response.
    fn send(
        &mut self,
        req: crate::SendMsg,
    ) -> impl std::future::Future<Output = Result<crate::SendResponse>> + Send {
        async move { crate::SendResponse::try_from(self.request(req.into()).await?) }
    }

    /// Send a message to an agent and receive a streamed response.
    fn stream(
        &mut self,
        req: crate::StreamMsg,
    ) -> impl Stream<Item = Result<stream_event::Event>> + Send + '_ {
        self.request_stream(req.into())
            .take_while(|r| {
                std::future::ready(!matches!(
                    r,
                    Ok(crate::ServerMessage {
                        msg: Some(server_message::Msg::Stream(crate::StreamEvent {
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
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::Ping(crate::Ping {})),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Get daemon stats including the active model name.
    fn get_stats(&mut self) -> impl std::future::Future<Output = Result<crate::Stats>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::GetStats(crate::GetStats {})),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Stats(stats)),
                } => Ok(stats),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// List all registered agents.
    fn list_agents(
        &mut self,
    ) -> impl std::future::Future<Output = Result<Vec<crate::AgentInfo>>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::ListAgents(crate::ListAgentsMsg {})),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::AgentList(crate::AgentList { agents })),
                } => Ok(agents),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Resolve an agent name to its record — the one name-keyed call.
    /// Every other agent operation takes the ULID in the reply.
    fn get_agent(
        &mut self,
        name: String,
    ) -> impl std::future::Future<Output = Result<crate::AgentInfo>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::GetAgent(crate::GetAgentMsg { name })),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::AgentInfo(info)),
                } => Ok(info),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Create an agent from JSON config and system prompt.
    fn create_agent(
        &mut self,
        name: String,
        config: String,
    ) -> impl std::future::Future<Output = Result<crate::AgentInfo>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::CreateAgent(crate::CreateAgentMsg {
                        name,
                        config,
                    })),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::AgentInfo(info)),
                } => Ok(info),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Update an agent from a JSON merge patch over its stored config.
    fn update_agent(
        &mut self,
        agent: String,
        config: String,
    ) -> impl std::future::Future<Output = Result<crate::AgentInfo>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::UpdateAgent(crate::UpdateAgentMsg {
                        agent,
                        config,
                    })),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::AgentInfo(info)),
                } => Ok(info),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Delete an agent, and every session it owns.
    fn delete_agent(
        &mut self,
        agent: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::DeleteAgent(crate::DeleteAgentMsg {
                        agent,
                    })),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Rename an agent. The agent's stored ULID stays stable.
    fn rename_agent(
        &mut self,
        agent: String,
        new_name: String,
    ) -> impl std::future::Future<Output = Result<crate::AgentInfo>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::RenameAgent(crate::RenameAgentMsg {
                        agent,
                        new_name,
                    })),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::AgentInfo(info)),
                } => Ok(info),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
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
    ) -> impl std::future::Future<Output = Result<Vec<crate::ConversationInfo>>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::ListConversations(
                        crate::ListConversationsMsg { agent, sender },
                    )),
                })
                .await?
            {
                crate::ServerMessage {
                    msg:
                        Some(server_message::Msg::ConversationList(crate::ConversationList {
                            conversations,
                        })),
                } => Ok(conversations),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
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
    ) -> impl std::future::Future<Output = Result<crate::ConversationHistory>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::GetConversationHistory(
                        crate::GetConversationHistoryMsg { file_path },
                    )),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::ConversationHistory(history)),
                } => Ok(history),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
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
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::DeleteConversation(
                        crate::DeleteConversationMsg { file_path },
                    )),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// List MCPs declared by agents. `agent` empty = union view across
    /// every registered agent.
    fn list_mcps(
        &mut self,
        agent: String,
    ) -> impl std::future::Future<Output = Result<Vec<crate::McpInfo>>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::ListMcps(crate::ListMcpsMsg { agent })),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::McpList(crate::McpList { mcps })),
                } => Ok(mcps),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => anyhow::bail!("server error ({code}): {message}"),
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Add or replace an MCP in the given agent's `mcps` list.
    fn upsert_mcp(
        &mut self,
        agent: String,
        config: String,
    ) -> impl std::future::Future<Output = Result<crate::McpInfo>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::UpsertMcp(crate::UpsertMcpMsg {
                        agent,
                        config,
                    })),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::McpInfo(info)),
                } => Ok(info),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => anyhow::bail!("server error ({code}): {message}"),
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Remove an MCP from the given agent's `mcps` list.
    fn delete_mcp(
        &mut self,
        agent: String,
        name: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::DeleteMcp(crate::DeleteMcpMsg {
                        agent,
                        name,
                    })),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => anyhow::bail!("server error ({code}): {message}"),
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Respawn the peer backing an already-registered MCP, leaving the
    /// stored config alone. Returns its post-reconnect status.
    fn reconnect_mcp(
        &mut self,
        agent: String,
        name: String,
    ) -> impl std::future::Future<Output = Result<crate::McpInfo>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::ReconnectMcp(crate::ReconnectMcpMsg {
                        agent,
                        name,
                    })),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::McpInfo(info)),
                } => Ok(info),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
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
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::SetActiveModel(
                        crate::SetActiveModelMsg { model },
                    )),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => anyhow::bail!("server error ({code}): {message}"),
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// List all available skills with enabled state.
    fn list_skills(
        &mut self,
    ) -> impl std::future::Future<Output = Result<Vec<crate::SkillInfo>>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::ListSkills(crate::ListSkillsMsg {})),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::SkillList(crate::SkillList { skills })),
                } => Ok(skills),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Ranked excerpts from past conversations. `agent` and `sender`
    /// are filters; empty means unrestricted.
    fn search_sessions(
        &mut self,
        req: crate::SearchSessionsMsg,
    ) -> impl std::future::Future<Output = Result<Vec<crate::SessionHit>>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::SearchSessions(req)),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::SessionHits(list)),
                } => Ok(list.hits),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Load one skill's instructions by name.
    fn get_skill(
        &mut self,
        name: String,
    ) -> impl std::future::Future<Output = Result<crate::SkillBody>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::GetSkill(crate::GetSkillMsg { name })),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::SkillBody(skill)),
                } => Ok(skill),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// List all resolved models with provider and active state.
    fn list_models(
        &mut self,
    ) -> impl std::future::Future<Output = Result<Vec<crate::ModelInfo>>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::ListModels(crate::ListModelsMsg {})),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::ModelList(crate::ModelList { models })),
                } => Ok(models),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Create an event bus subscription.
    fn subscribe_event(
        &mut self,
        source: String,
        target_agent: String,
        once: bool,
    ) -> impl std::future::Future<Output = Result<crate::SubscriptionInfo>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::SubscribeEvent(
                        crate::SubscribeEventMsg {
                            source,
                            target_agent,
                            once,
                        },
                    )),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::SubscriptionInfo(info)),
                } => Ok(info),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Remove an event bus subscription.
    fn unsubscribe_event(
        &mut self,
        id: u64,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::UnsubscribeEvent(
                        crate::UnsubscribeEventMsg { id },
                    )),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// List all event bus subscriptions.
    fn list_subscriptions(
        &mut self,
    ) -> impl std::future::Future<Output = Result<Vec<crate::SubscriptionInfo>>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::ListSubscriptions(
                        crate::ListSubscriptionsMsg {},
                    )),
                })
                .await?
            {
                crate::ServerMessage {
                    msg:
                        Some(server_message::Msg::SubscriptionList(crate::SubscriptionList {
                            subscriptions,
                        })),
                } => Ok(subscriptions),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Publish an event to the bus.
    fn publish_event(
        &mut self,
        source: String,
        payload: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::PublishEvent(crate::PublishEventMsg {
                        source,
                        payload,
                    })),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// List active (in-memory) conversations on the daemon.
    fn list_active_conversations(
        &mut self,
        agent: String,
        sender: String,
    ) -> impl std::future::Future<Output = Result<Vec<crate::ActiveConversationInfo>>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::ListActiveConversations(
                        crate::ListActiveConversationsMsg { agent, sender },
                    )),
                })
                .await?
            {
                crate::ServerMessage {
                    msg:
                        Some(server_message::Msg::ActiveConversations(crate::ActiveConversationList {
                            conversations,
                        })),
                } => Ok(conversations),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => anyhow::bail!("server error ({code}): {message}"),
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Kill an active conversation by (agent, sender). Returns true if it existed.
    fn kill_conversation(
        &mut self,
        agent: String,
        sender: String,
    ) -> impl std::future::Future<Output = Result<bool>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::Kill(crate::KillMsg { agent, sender })),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(true),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code: 404, .. })),
                } => Ok(false),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => anyhow::bail!("server error ({code}): {message}"),
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Compact a conversation's history into a summary.
    fn compact_conversation(
        &mut self,
        agent: String,
        sender: String,
    ) -> impl std::future::Future<Output = Result<String>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::Compact(crate::CompactMsg {
                        agent,
                        sender,
                    })),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Compact(crate::CompactResponse { summary })),
                } => Ok(summary),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => anyhow::bail!("server error ({code}): {message}"),
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Get the daemon's config snapshot as a JSON string.
    fn get_config(&mut self) -> impl std::future::Future<Output = Result<String>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::GetConfig(crate::GetConfig {})),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Config(crate::ConfigMsg { config })),
                } => Ok(config),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => anyhow::bail!("server error ({code}): {message}"),
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Hot-reload daemon runtime from disk.
    fn reload(&mut self) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::Reload(crate::ReloadMsg {})),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => anyhow::bail!("server error ({code}): {message}"),
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Subscribe to all agent events. Returns a stream that ends when the
    /// connection drops.
    fn subscribe_events(&mut self) -> impl Stream<Item = Result<crate::AgentEventMsg>> + Send + '_ {
        self.request_stream(crate::ClientMessage {
            msg: Some(client_message::Msg::SubscribeEvents(
                crate::SubscribeEvents {},
            )),
        })
        .filter_map(|r| async {
            match r {
                Ok(crate::ServerMessage {
                    msg: Some(server_message::Msg::AgentEvent(e)),
                }) => Some(Ok(e)),
                Ok(crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                }) => Some(Err(anyhow::anyhow!("server error ({code}): {message}"))),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            }
        })
    }

    /// Subscribe to MCP lifecycle events.
    fn subscribe_mcp_events(
        &mut self,
    ) -> impl Stream<Item = Result<crate::McpEventMsg>> + Send + '_ {
        self.request_stream(crate::ClientMessage {
            msg: Some(client_message::Msg::SubscribeMcpEvents(
                crate::SubscribeMcpEventsMsg {},
            )),
        })
        .filter_map(|r| async {
            match r {
                Ok(crate::ServerMessage {
                    msg: Some(server_message::Msg::McpEvent(e)),
                }) => Some(Ok(e)),
                Ok(crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                }) => Some(Err(anyhow::anyhow!("server error ({code}): {message}"))),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            }
        })
    }

    /// Cancel the in-flight stream for a session.
    fn cancel_stream(
        &mut self,
        agent: String,
        sender: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(crate::ClientMessage {
                    msg: Some(client_message::Msg::CancelStream(crate::CancelStreamMsg {
                        agent,
                        sender,
                    })),
                })
                .await?
            {
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                crate::ServerMessage {
                    msg: Some(server_message::Msg::Error(crate::ErrorMsg { code, message })),
                } => anyhow::bail!("server error ({code}): {message}"),
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Steer a session: cancel its in-flight stream, then start a new
    /// turn with `content`. Nothing running isn't an error — the cancel
    /// no-ops and the message streams normally.
    fn steer_session(
        &mut self,
        agent: String,
        sender: String,
        content: String,
    ) -> impl Stream<Item = Result<stream_event::Event>> + Send + '_ {
        async_stream::stream! {
            let _ = self.cancel_stream(agent.clone(), sender.clone()).await;
            let mut stream = std::pin::pin!(self.stream(crate::StreamMsg {
                agent,
                content,
                sender: Some(sender),
                ..Default::default()
            }));
            while let Some(event) = stream.next().await {
                yield event;
            }
        }
    }
}
