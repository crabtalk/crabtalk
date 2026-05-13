//! Client trait — transport primitives plus typed provided methods.

use crate::protocol::message::*;
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

    /// Create an agent from JSON config and system prompt.
    fn create_agent(
        &mut self,
        name: String,
        config: String,
        prompt: String,
    ) -> impl std::future::Future<Output = Result<AgentInfo>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::CreateAgent(CreateAgentMsg {
                        name,
                        config,
                        prompt,
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

    /// Update an agent from JSON config and optional system prompt.
    fn update_agent(
        &mut self,
        name: String,
        config: String,
        prompt: String,
    ) -> impl std::future::Future<Output = Result<AgentInfo>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::UpdateAgent(UpdateAgentMsg {
                        name,
                        config,
                        prompt,
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

    /// Rename an agent. The agent's stored ULID stays stable.
    fn rename_agent(
        &mut self,
        old_name: String,
        new_name: String,
    ) -> impl std::future::Future<Output = Result<AgentInfo>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::RenameAgent(RenameAgentMsg {
                        old_name,
                        new_name,
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

    /// List MCPs declared by agents. `agent` empty = union view across
    /// every registered agent.
    fn list_mcps(
        &mut self,
        agent: String,
    ) -> impl std::future::Future<Output = Result<Vec<McpInfo>>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::ListMcps(ListMcpsMsg { agent })),
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

    /// Add or replace an MCP in the given agent's `mcps` list.
    fn upsert_mcp(
        &mut self,
        agent: String,
        config: String,
    ) -> impl std::future::Future<Output = Result<McpInfo>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::UpsertMcp(UpsertMcpMsg {
                        agent,
                        config,
                    })),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::McpInfo(info)),
                } => Ok(info),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
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
                .request(ClientMessage {
                    msg: Some(client_message::Msg::DeleteMcp(DeleteMcpMsg { agent, name })),
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

    /// Create an event bus subscription.
    fn subscribe_event(
        &mut self,
        source: String,
        target_agent: String,
        once: bool,
    ) -> impl std::future::Future<Output = Result<SubscriptionInfo>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::SubscribeEvent(SubscribeEventMsg {
                        source,
                        target_agent,
                        once,
                    })),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::SubscriptionInfo(info)),
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

    /// Remove an event bus subscription.
    fn unsubscribe_event(
        &mut self,
        id: u64,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::UnsubscribeEvent(UnsubscribeEventMsg {
                        id,
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

    /// List all event bus subscriptions.
    fn list_subscriptions(
        &mut self,
    ) -> impl std::future::Future<Output = Result<Vec<SubscriptionInfo>>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::ListSubscriptions(
                        ListSubscriptionsMsg {},
                    )),
                })
                .await?
            {
                ServerMessage {
                    msg:
                        Some(server_message::Msg::SubscriptionList(SubscriptionList { subscriptions })),
                } => Ok(subscriptions),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
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
                .request(ClientMessage {
                    msg: Some(client_message::Msg::PublishEvent(PublishEventMsg {
                        source,
                        payload,
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

    /// Deliver a user reply to a pending `ask_user` tool call.
    fn reply_to_ask(
        &mut self,
        agent: String,
        sender: String,
        content: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(ClientMessage::from(ReplyToAsk {
                    agent,
                    sender,
                    content,
                }))
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

    /// Deliver a client-side tool result for a pending forwarded call. The
    /// daemon resolves the pending entry keyed by `(conversation_id, call_id)`.
    fn reply_to_tool(
        &mut self,
        conversation_id: u64,
        call_id: String,
        output: String,
        is_error: bool,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            let msg = ClientMessage {
                msg: Some(client_message::Msg::ReplyToTool(ReplyToTool {
                    conversation_id,
                    call_id,
                    output,
                    is_error,
                })),
            };
            match self.request(msg).await? {
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

    /// List active (in-memory) conversations on the daemon.
    fn list_active_conversations(
        &mut self,
        agent: String,
        sender: String,
    ) -> impl std::future::Future<Output = Result<Vec<ActiveConversationInfo>>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::ListActiveConversations(
                        ListActiveConversationsMsg { agent, sender },
                    )),
                })
                .await?
            {
                ServerMessage {
                    msg:
                        Some(server_message::Msg::ActiveConversations(ActiveConversationList {
                            conversations,
                        })),
                } => Ok(conversations),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
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
                .request(ClientMessage {
                    msg: Some(client_message::Msg::Kill(KillMsg { agent, sender })),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(true),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code: 404, .. })),
                } => Ok(false),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
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
                .request(ClientMessage {
                    msg: Some(client_message::Msg::Compact(CompactMsg { agent, sender })),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::Compact(CompactResponse { summary })),
                } => Ok(summary),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => anyhow::bail!("server error ({code}): {message}"),
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Get the daemon's config snapshot as a JSON string.
    fn get_config(&mut self) -> impl std::future::Future<Output = Result<String>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::GetConfig(GetConfig {})),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::Config(ConfigMsg { config })),
                } => Ok(config),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => anyhow::bail!("server error ({code}): {message}"),
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Hot-reload daemon runtime from disk.
    fn reload(&mut self) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::Reload(ReloadMsg {})),
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

    /// Subscribe to all agent events. Returns a stream that ends when the
    /// connection drops.
    fn subscribe_events(&mut self) -> impl Stream<Item = Result<AgentEventMsg>> + Send + '_ {
        self.request_stream(ClientMessage {
            msg: Some(client_message::Msg::SubscribeEvents(SubscribeEvents {})),
        })
        .filter_map(|r| async {
            match r {
                Ok(ServerMessage {
                    msg: Some(server_message::Msg::AgentEvent(e)),
                }) => Some(Ok(e)),
                Ok(ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                }) => Some(Err(anyhow::anyhow!("server error ({code}): {message}"))),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            }
        })
    }

    /// Subscribe to MCP lifecycle events.
    fn subscribe_mcp_events(&mut self) -> impl Stream<Item = Result<McpEventMsg>> + Send + '_ {
        self.request_stream(ClientMessage {
            msg: Some(client_message::Msg::SubscribeMcpEvents(
                SubscribeMcpEventsMsg {},
            )),
        })
        .filter_map(|r| async {
            match r {
                Ok(ServerMessage {
                    msg: Some(server_message::Msg::McpEvent(e)),
                }) => Some(Ok(e)),
                Ok(ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                }) => Some(Err(anyhow::anyhow!("server error ({code}): {message}"))),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            }
        })
    }

    /// Inject a user message into an active stream (steering).
    fn steer_session(
        &mut self,
        agent: String,
        sender: String,
        content: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::SteerSession(SteerSessionMsg {
                        agent,
                        sender,
                        content,
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
}
