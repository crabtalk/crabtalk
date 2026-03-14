//! Gateway runner — connects to walrusd via Unix domain socket or TCP.

use anyhow::Result;
use futures_core::Stream;
use futures_util::StreamExt;
use socket::{ClientConfig, Connection, WalrusClient};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use tcp::TcpConnection;
use wcore::protocol::{
    api::Client,
    message::{
        ApproveMsg, ClientMessage, HubAction, HubMsg, KillMsg, KillTaskMsg, SendMsg, SendResponse,
        ServerMessage, SessionInfo, StreamMsg, TaskInfo, client_message, download_event,
        server_message, stream_event,
    },
};

/// A typed chunk from the streaming response.
pub enum OutputChunk {
    /// Regular text content.
    Text(String),
    /// Thinking/reasoning content (displayed dimmed).
    Thinking(String),
    /// Status message (tool calls, etc.).
    Status(String),
}

/// Transport-agnostic connection to walrusd.
enum Transport {
    Uds(Connection),
    Tcp(TcpConnection),
}

impl Transport {
    async fn request(&mut self, msg: ClientMessage) -> Result<ServerMessage> {
        match self {
            Self::Uds(c) => c.request(msg).await,
            Self::Tcp(c) => c.request(msg).await,
        }
    }

    fn request_stream(
        &mut self,
        msg: ClientMessage,
    ) -> impl Stream<Item = Result<ServerMessage>> + Send + '_ {
        async_stream::try_stream! {
            match self {
                Self::Uds(c) => {
                    let s = c.request_stream(msg);
                    tokio::pin!(s);
                    while let Some(item) = s.next().await {
                        yield item?;
                    }
                }
                Self::Tcp(c) => {
                    let s = c.request_stream(msg);
                    tokio::pin!(s);
                    while let Some(item) = s.next().await {
                        yield item?;
                    }
                }
            }
        }
    }
}

/// Runs agents via a walrusd connection (UDS or TCP).
pub struct Runner {
    transport: Transport,
}

impl Runner {
    /// Connect to walrusd via Unix domain socket.
    pub async fn connect(socket_path: &Path) -> Result<Self> {
        let config = ClientConfig {
            socket_path: socket_path.to_path_buf(),
        };
        let client = WalrusClient::new(config);
        let connection = client.connect().await?;
        Ok(Self {
            transport: Transport::Uds(connection),
        })
    }

    /// Connect to walrusd via TCP.
    pub async fn connect_tcp(port: u16) -> Result<Self> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let connection = TcpConnection::connect(addr).await?;
        Ok(Self {
            transport: Transport::Tcp(connection),
        })
    }

    /// Send a one-shot message and return the response content.
    pub async fn send(&mut self, agent: &str, content: &str) -> Result<String> {
        let resp = self
            .transport
            .request(ClientMessage::from(SendMsg {
                agent: agent.to_string(),
                content: content.to_string(),
                session: None,
                sender: None,
            }))
            .await?;
        let resp = SendResponse::try_from(resp)?;
        Ok(resp.content)
    }

    /// Stream a response, yielding typed output chunks.
    pub fn stream<'a>(
        &'a mut self,
        agent: &'a str,
        content: &'a str,
    ) -> impl Stream<Item = Result<OutputChunk>> + Send + 'a {
        self.transport
            .request_stream(ClientMessage::from(StreamMsg {
                agent: agent.to_string(),
                content: content.to_string(),
                session: None,
                sender: None,
            }))
            .take_while(|r| {
                std::future::ready(!matches!(
                    r,
                    Ok(ServerMessage {
                        msg: Some(server_message::Msg::Stream(e))
                    }) if matches!(e.event, Some(stream_event::Event::End(_)))
                ))
            })
            .filter_map(|result| async {
                match result {
                    Ok(ServerMessage {
                        msg: Some(server_message::Msg::Stream(e)),
                    }) => match &e.event {
                        Some(stream_event::Event::Chunk(c)) => {
                            Some(Ok(OutputChunk::Text(c.content.clone())))
                        }
                        Some(stream_event::Event::Thinking(t)) => {
                            Some(Ok(OutputChunk::Thinking(t.content.clone())))
                        }
                        Some(stream_event::Event::ToolStart(ts)) => {
                            let names: Vec<_> = ts.calls.iter().map(|c| c.name.as_str()).collect();
                            Some(Ok(OutputChunk::Status(format!(
                                "\n[calling {}...]\n",
                                names.join(", ")
                            ))))
                        }
                        Some(stream_event::Event::ToolResult(_)) => None,
                        Some(stream_event::Event::ToolsComplete(_)) => {
                            Some(Ok(OutputChunk::Status("[done]\n".to_string())))
                        }
                        Some(stream_event::Event::Start(_)) => None,
                        Some(stream_event::Event::End(_)) => None,
                        None => None,
                    },
                    Ok(ServerMessage {
                        msg: Some(server_message::Msg::Error(e)),
                    }) => Some(Err(anyhow::anyhow!(
                        "server error ({}): {}",
                        e.code,
                        e.message
                    ))),
                    Ok(_) => None,
                    Err(e) => Some(Err(e)),
                }
            })
    }

    /// Send a hub install/uninstall request and return a stream of progress events.
    pub fn hub_stream(
        &mut self,
        package: &str,
        action: HubAction,
    ) -> impl Stream<Item = Result<download_event::Event>> + '_ {
        self.transport
            .request_stream(ClientMessage {
                msg: Some(client_message::Msg::Hub(HubMsg {
                    package: package.to_string(),
                    action: action.into(),
                })),
            })
            .take_while(|r| {
                std::future::ready(!matches!(
                    r,
                    Ok(ServerMessage {
                        msg: Some(server_message::Msg::Download(e))
                    }) if matches!(e.event, Some(download_event::Event::Completed(_)))
                ))
            })
            .map(|r: Result<ServerMessage>| r.and_then(download_event::Event::try_from))
    }

    /// List active sessions on the daemon.
    pub async fn list_sessions(&mut self) -> Result<Vec<SessionInfo>> {
        let msg = ClientMessage {
            msg: Some(client_message::Msg::Sessions(Default::default())),
        };
        match self.transport.request(msg).await? {
            ServerMessage {
                msg: Some(server_message::Msg::Sessions(sl)),
            } => Ok(sl.sessions),
            ServerMessage {
                msg: Some(server_message::Msg::Error(e)),
            } => {
                anyhow::bail!("server error ({}): {}", e.code, e.message)
            }
            other => anyhow::bail!("unexpected response: {other:?}"),
        }
    }

    /// Kill (close) a session by ID. Returns true if it existed.
    pub async fn kill_session(&mut self, session: u64) -> Result<bool> {
        let msg = ClientMessage {
            msg: Some(client_message::Msg::Kill(KillMsg { session })),
        };
        match self.transport.request(msg).await? {
            ServerMessage {
                msg: Some(server_message::Msg::Pong(_)),
            } => Ok(true),
            ServerMessage {
                msg: Some(server_message::Msg::Error(e)),
            } if e.code == 404 => Ok(false),
            ServerMessage {
                msg: Some(server_message::Msg::Error(e)),
            } => {
                anyhow::bail!("server error ({}): {}", e.code, e.message)
            }
            other => anyhow::bail!("unexpected response: {other:?}"),
        }
    }

    /// List tasks in the task registry.
    pub async fn list_tasks(&mut self) -> Result<Vec<TaskInfo>> {
        let msg = ClientMessage {
            msg: Some(client_message::Msg::Tasks(Default::default())),
        };
        match self.transport.request(msg).await? {
            ServerMessage {
                msg: Some(server_message::Msg::Tasks(tl)),
            } => Ok(tl.tasks),
            ServerMessage {
                msg: Some(server_message::Msg::Error(e)),
            } => {
                anyhow::bail!("server error ({}): {}", e.code, e.message)
            }
            other => anyhow::bail!("unexpected response: {other:?}"),
        }
    }

    /// Kill (cancel) a task by ID. Returns true if it existed.
    pub async fn kill_task(&mut self, task_id: u64) -> Result<bool> {
        let msg = ClientMessage {
            msg: Some(client_message::Msg::KillTask(KillTaskMsg { task_id })),
        };
        match self.transport.request(msg).await? {
            ServerMessage {
                msg: Some(server_message::Msg::Pong(_)),
            } => Ok(true),
            ServerMessage {
                msg: Some(server_message::Msg::Error(e)),
            } if e.code == 404 => Ok(false),
            ServerMessage {
                msg: Some(server_message::Msg::Error(e)),
            } => {
                anyhow::bail!("server error ({}): {}", e.code, e.message)
            }
            other => anyhow::bail!("unexpected response: {other:?}"),
        }
    }

    /// Approve a blocked task. Returns true if the task was blocked and approved.
    pub async fn approve_task(&mut self, task_id: u64, response: String) -> Result<bool> {
        let msg = ClientMessage {
            msg: Some(client_message::Msg::Approve(ApproveMsg {
                task_id,
                response,
            })),
        };
        match self.transport.request(msg).await? {
            ServerMessage {
                msg: Some(server_message::Msg::Pong(_)),
            } => Ok(true),
            ServerMessage {
                msg: Some(server_message::Msg::Error(e)),
            } if e.code == 404 => Ok(false),
            ServerMessage {
                msg: Some(server_message::Msg::Error(e)),
            } => {
                anyhow::bail!("server error ({}): {}", e.code, e.message)
            }
            other => anyhow::bail!("unexpected response: {other:?}"),
        }
    }
}
