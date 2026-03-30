//! Client trait — transport primitives plus typed provided methods.

use crate::protocol::message::{
    AgentInfo, AgentList, ClientMessage, ConfigMsg, CreateAgentMsg, DeleteAgentMsg, ErrorMsg,
    GetAgentMsg, GetConfig, ListAgentsMsg, Ping, SendMsg, SendResponse, ServerMessage,
    SetConfigMsg, StreamEvent, StreamMsg, UpdateAgentMsg, client_message, server_message,
    stream_event,
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

    /// Get the full daemon config as JSON.
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
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Replace the full daemon config from JSON.
    fn set_config(
        &mut self,
        config: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::SetConfig(SetConfigMsg { config })),
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
}
