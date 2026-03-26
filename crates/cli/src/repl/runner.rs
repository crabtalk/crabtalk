//! Gateway runner — connects to crabtalk daemon via Unix domain socket or TCP.

use anyhow::Result;
use futures_core::Stream;
use futures_util::StreamExt;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use transport::tcp::TcpConnection;
use transport::uds::{ClientConfig, Connection, CrabtalkClient};
use wcore::protocol::{
    api::Client,
    message::{
        AgentEventMsg, AskQuestion, ClientMessage, ConfigMsg, GetConfig, KillMsg, ReplyToAsk,
        ServerMessage, SessionInfo, StreamMsg, SubscribeEvents, client_message, server_message,
        stream_event,
    },
};

/// A typed chunk from the streaming response.
pub enum OutputChunk {
    /// Regular text content.
    Text(String),
    /// Thinking/reasoning content (displayed dimmed).
    Thinking(String),
    /// Tool execution started with these tool calls (name, arguments JSON).
    ToolStart(Vec<(String, String)>),
    /// Tool result returned (tool name, result content).
    ToolResult(String, String),
    /// Tool execution completed (true = success, false = failure).
    ToolDone(bool),
    /// Agent is asking the user structured questions. Carries questions and session ID.
    AskUser {
        questions: Vec<AskQuestion>,
        session: u64,
    },
}

/// Transport-agnostic connection to the crabtalk daemon.
enum Transport {
    Uds(Connection),
    Tcp(TcpConnection),
}

/// Dispatch a method call to the inner connection regardless of variant.
macro_rules! dispatch {
    ($self:expr, |$c:ident| $body:expr) => {
        match $self {
            Transport::Uds($c) => $body,
            Transport::Tcp($c) => $body,
        }
    };
}

impl Transport {
    async fn request(&mut self, msg: ClientMessage) -> Result<ServerMessage> {
        dispatch!(self, |c| c.request(msg).await)
    }

    fn request_stream(
        &mut self,
        msg: ClientMessage,
    ) -> impl Stream<Item = Result<ServerMessage>> + Send + '_ {
        async_stream::try_stream! {
            dispatch!(self, |c| {
                let s = c.request_stream(msg);
                tokio::pin!(s);
                while let Some(item) = s.next().await {
                    yield item?;
                }
            });
        }
    }
}

/// How to reconnect to the daemon (for sending ReplyToAsk on a separate connection).
#[derive(Clone)]
pub enum ConnectionInfo {
    Uds(PathBuf),
    Tcp(u16),
}

/// Runs agents via a crabtalk daemon connection (UDS or TCP).
pub struct Runner {
    transport: Transport,
    conn_info: ConnectionInfo,
}

impl Runner {
    /// Connect to crabtalk daemon via Unix domain socket.
    pub async fn connect(socket_path: &Path) -> Result<Self> {
        let config = ClientConfig {
            socket_path: socket_path.to_path_buf(),
        };
        let client = CrabtalkClient::new(config);
        let connection = client.connect().await?;
        Ok(Self {
            transport: Transport::Uds(connection),
            conn_info: ConnectionInfo::Uds(socket_path.to_path_buf()),
        })
    }

    /// Connect to crabtalk daemon via TCP.
    pub async fn connect_tcp(port: u16) -> Result<Self> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let connection = TcpConnection::connect(addr).await?;
        Ok(Self {
            transport: Transport::Tcp(connection),
            conn_info: ConnectionInfo::Tcp(port),
        })
    }

    /// Create a new connection from existing connection info.
    pub async fn connect_from(info: &ConnectionInfo) -> Result<Self> {
        match info {
            ConnectionInfo::Uds(path) => Self::connect(path).await,
            ConnectionInfo::Tcp(port) => Self::connect_tcp(*port).await,
        }
    }

    /// Connection info for creating separate connections (e.g. for ReplyToAsk).
    pub fn conn_info(&self) -> &ConnectionInfo {
        &self.conn_info
    }

    /// Stream a response, yielding typed output chunks.
    ///
    /// If `cwd` is `Some`, the agent session uses that directory for tool
    /// execution instead of the process's current working directory.
    pub fn stream<'a>(
        &'a mut self,
        agent: &'a str,
        content: &'a str,
        cwd: Option<&'a Path>,
        new_chat: bool,
        resume_file: Option<String>,
        sender: Option<String>,
    ) -> impl Stream<Item = Result<OutputChunk>> + Send + 'a {
        let cwd = cwd.map(|p| p.to_string_lossy().into_owned()).or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        });
        self.transport
            .request_stream(ClientMessage::from(StreamMsg {
                agent: agent.to_string(),
                content: content.to_string(),
                session: None,
                sender,
                cwd,
                new_chat,
                resume_file,
            }))
            .take_while(|r| {
                std::future::ready(!matches!(
                    r,
                    Ok(ServerMessage {
                        msg: Some(server_message::Msg::Stream(e))
                    }) if matches!(&e.event, Some(stream_event::Event::End(end)) if end.error.is_empty())
                ))
            })
            .scan(0u64, |session_id, result| {
                let chunk = match result {
                    Ok(ServerMessage {
                        msg: Some(server_message::Msg::Stream(e)),
                    }) => match &e.event {
                        Some(stream_event::Event::Start(s)) => {
                            *session_id = s.session;
                            None
                        }
                        Some(stream_event::Event::Chunk(c)) => {
                            Some(Ok(OutputChunk::Text(c.content.clone())))
                        }
                        Some(stream_event::Event::Thinking(t)) => {
                            Some(Ok(OutputChunk::Thinking(t.content.clone())))
                        }
                        Some(stream_event::Event::ToolStart(ts)) => {
                            let calls: Vec<_> = ts
                                .calls
                                .iter()
                                .map(|c| (c.name.clone(), c.arguments.clone()))
                                .collect();
                            Some(Ok(OutputChunk::ToolStart(calls)))
                        }
                        Some(stream_event::Event::ToolResult(tr)) => Some(Ok(
                            OutputChunk::ToolResult(tr.call_id.clone(), tr.output.clone()),
                        )),
                        Some(stream_event::Event::ToolsComplete(_)) => {
                            Some(Ok(OutputChunk::ToolDone(true)))
                        }
                        Some(stream_event::Event::AskUser(ask)) => Some(Ok(OutputChunk::AskUser {
                            questions: ask.questions.clone(),
                            session: *session_id,
                        })),
                        Some(stream_event::Event::End(end)) if !end.error.is_empty() => {
                            Some(Err(anyhow::anyhow!("{}", end.error)))
                        }
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
                };
                std::future::ready(Some(chunk))
            })
            .filter_map(std::future::ready)
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

    /// Trigger a daemon reload. Returns Ok(()) on success.
    pub async fn reload(&mut self) -> Result<()> {
        let msg = ClientMessage {
            msg: Some(client_message::Msg::Reload(Default::default())),
        };
        match self.transport.request(msg).await? {
            ServerMessage {
                msg: Some(server_message::Msg::Pong(_)),
            } => Ok(()),
            ServerMessage {
                msg: Some(server_message::Msg::Error(e)),
            } => {
                anyhow::bail!("server error ({}): {}", e.code, e.message)
            }
            other => anyhow::bail!("unexpected response: {other:?}"),
        }
    }

    /// Subscribe to agent events. Returns a stream of `AgentEventMsg`.
    pub fn subscribe_events(&mut self) -> impl Stream<Item = Result<AgentEventMsg>> + Send + '_ {
        self.transport
            .request_stream(ClientMessage {
                msg: Some(client_message::Msg::SubscribeEvents(SubscribeEvents {})),
            })
            .filter_map(|r| async {
                match r {
                    Ok(ServerMessage {
                        msg: Some(server_message::Msg::AgentEvent(e)),
                    }) => Some(Ok(e)),
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

    /// Get the daemon config as JSON string.
    pub async fn get_config(&mut self) -> Result<String> {
        let msg = ClientMessage {
            msg: Some(client_message::Msg::GetConfig(GetConfig {})),
        };
        match self.transport.request(msg).await? {
            ServerMessage {
                msg: Some(server_message::Msg::Config(ConfigMsg { config })),
            } => Ok(config),
            ServerMessage {
                msg: Some(server_message::Msg::Error(e)),
            } => {
                anyhow::bail!("server error ({}): {}", e.code, e.message)
            }
            other => anyhow::bail!("unexpected response: {other:?}"),
        }
    }
}

/// Send a `ReplyToAsk` to the daemon on a temporary connection.
pub async fn send_reply(conn_info: &ConnectionInfo, session: u64, content: String) -> Result<()> {
    let msg = ClientMessage::from(ReplyToAsk { session, content });
    match conn_info {
        ConnectionInfo::Uds(path) => {
            let client = CrabtalkClient::new(ClientConfig {
                socket_path: path.clone(),
            });
            let mut conn = client.connect().await?;
            conn.request(msg).await?;
        }
        ConnectionInfo::Tcp(port) => {
            let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, *port));
            let mut conn = TcpConnection::connect(addr).await?;
            conn.request(msg).await?;
        }
    }
    Ok(())
}
