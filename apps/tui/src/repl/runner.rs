//! Gateway runner — connects to crabtalk daemon via Unix domain socket or TCP.

use anyhow::Result;
use futures_core::Stream;
use futures_util::StreamExt;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use wcore::protocol::{
    api::Client,
    message::{
        ActiveConversationInfo, AgentEventMsg, AskQuestion, ClientMessage, InstallPluginMsg,
        KillMsg, ListActiveConversationsMsg, PluginEvent, ReplyToAsk, ServerMessage, StreamMsg,
        SubscribeEvents, UninstallPluginMsg, client_message, plugin_event, server_message,
        stream_event,
    },
};

/// A typed chunk from the streaming response.
pub enum OutputChunk {
    /// A text segment is starting.
    TextStart,
    /// Regular text content delta.
    Text(String),
    /// The current text segment has ended.
    TextEnd,
    /// A thinking segment is starting.
    ThinkingStart,
    /// Thinking/reasoning content delta (displayed dimmed).
    Thinking(String),
    /// The current thinking segment has ended.
    ThinkingEnd,
    /// Tool execution started with these tool calls (name, arguments JSON).
    ToolStart(Vec<(String, String)>),
    /// Tool result returned (tool name, result content).
    ToolResult(String, String),
    /// Tool execution completed (true = success, false = failure).
    ToolDone(bool),
    /// Agent is asking the user structured questions. Carries questions and agent identity.
    AskUser {
        questions: Vec<AskQuestion>,
        agent: String,
        sender: String,
    },
}

pub use transport::Transport;

/// How to reconnect to the daemon (for sending ReplyToAsk on a separate connection).
#[derive(Clone)]
pub enum ConnectionInfo {
    #[cfg(unix)]
    Uds(PathBuf),
    Tcp(u16),
}

/// Runs agents via a crabtalk daemon connection (UDS or TCP).
///
/// Implements `DerefMut<Target = Transport>` so all [`Client`] trait methods
/// (list_providers, set_provider, list_mcps, etc.) are callable directly.
pub struct Runner {
    transport: Transport,
    pub conn_info: ConnectionInfo,
}

impl std::ops::Deref for Runner {
    type Target = Transport;
    fn deref(&self) -> &Self::Target {
        &self.transport
    }
}

impl std::ops::DerefMut for Runner {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.transport
    }
}

impl Runner {
    /// Connect to crabtalk daemon via Unix domain socket.
    #[cfg(unix)]
    pub async fn connect(socket_path: &Path) -> Result<Self> {
        let config = transport::uds::ClientConfig {
            socket_path: socket_path.to_path_buf(),
        };
        let client = transport::uds::CrabtalkClient::new(config);
        let connection = client.connect().await?;
        Ok(Self {
            transport: Transport::Uds(connection),
            conn_info: ConnectionInfo::Uds(socket_path.to_path_buf()),
        })
    }

    /// Connect to crabtalk daemon via TCP.
    pub async fn connect_tcp(port: u16) -> Result<Self> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let connection = transport::tcp::TcpConnection::connect(addr).await?;
        Ok(Self {
            transport: Transport::Tcp(connection),
            conn_info: ConnectionInfo::Tcp(port),
        })
    }

    /// Create a new connection from existing connection info.
    pub async fn connect_from(info: &ConnectionInfo) -> Result<Self> {
        match info {
            #[cfg(unix)]
            ConnectionInfo::Uds(path) => Self::connect(path).await,
            ConnectionInfo::Tcp(port) => Self::connect_tcp(*port).await,
        }
    }

    /// Stream a response, yielding typed output chunks.
    ///
    /// If `cwd` is `Some`, the agent conversation uses that directory for tool
    /// execution instead of the process's current working directory.
    pub fn stream<'a>(
        &'a mut self,
        agent: &'a str,
        content: &'a str,
        cwd: Option<&'a Path>,
        sender: Option<String>,
    ) -> impl Stream<Item = Result<OutputChunk>> + Send + 'a {
        let cwd = cwd.map(|p| p.to_string_lossy().into_owned()).or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        });
        let agent_name = agent.to_string();
        let sender_name = sender.clone().unwrap_or_default();
        self.transport
            .request_stream(ClientMessage::from(StreamMsg {
                agent: agent.to_string(),
                content: content.to_string(),
                sender,
                cwd,
                guest: None,
                tool_choice: None,
            }))
            .take_while(|r| {
                std::future::ready(!matches!(
                    r,
                    Ok(ServerMessage {
                        msg: Some(server_message::Msg::Stream(e))
                    }) if matches!(&e.event, Some(stream_event::Event::End(end)) if end.error.is_empty())
                ))
            })
            .scan((agent_name, sender_name), |state, result| {
                let chunk = match result {
                    Ok(ServerMessage {
                        msg: Some(server_message::Msg::Stream(e)),
                    }) => match &e.event {
                        Some(stream_event::Event::Start(_)) => {
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
                            agent: state.0.clone(),
                            sender: state.1.clone(),
                        })),
                        Some(stream_event::Event::UserSteered(_)) => None,
                        Some(stream_event::Event::End(end)) if !end.error.is_empty() => {
                            Some(Err(anyhow::anyhow!("{}", end.error)))
                        }
                        Some(stream_event::Event::End(_)) => None,
                        Some(stream_event::Event::TextStart(_)) => {
                            Some(Ok(OutputChunk::TextStart))
                        }
                        Some(stream_event::Event::TextEnd(_)) => Some(Ok(OutputChunk::TextEnd)),
                        Some(stream_event::Event::ThinkingStart(_)) => {
                            Some(Ok(OutputChunk::ThinkingStart))
                        }
                        Some(stream_event::Event::ThinkingEnd(_)) => {
                            Some(Ok(OutputChunk::ThinkingEnd))
                        }
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

    /// List active conversations on the daemon.
    pub async fn list_active_conversations(&mut self) -> Result<Vec<ActiveConversationInfo>> {
        let msg = ClientMessage {
            msg: Some(client_message::Msg::ListActiveConversations(
                ListActiveConversationsMsg {
                    agent: String::new(),
                    sender: String::new(),
                },
            )),
        };
        match self.transport.request(msg).await? {
            ServerMessage {
                msg: Some(server_message::Msg::ActiveConversations(sl)),
            } => Ok(sl.conversations),
            ServerMessage {
                msg: Some(server_message::Msg::Error(e)),
            } => {
                anyhow::bail!("server error ({}): {}", e.code, e.message)
            }
            other => anyhow::bail!("unexpected response: {other:?}"),
        }
    }

    /// Kill (close) a conversation by (agent, sender). Returns true if it existed.
    pub async fn kill_conversation(&mut self, agent: &str, sender: &str) -> Result<bool> {
        let msg = ClientMessage {
            msg: Some(client_message::Msg::Kill(KillMsg {
                agent: agent.to_string(),
                sender: sender.to_string(),
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

    /// Install a plugin, streaming progress events.
    pub fn install_plugin<'a>(
        &'a mut self,
        plugin: &str,
        branch: &str,
        path: &str,
        force: bool,
    ) -> impl Stream<Item = Result<plugin_event::Event>> + Send + 'a {
        self.transport
            .request_stream(ClientMessage {
                msg: Some(client_message::Msg::InstallPlugin(InstallPluginMsg {
                    plugin: plugin.to_string(),
                    branch: branch.to_string(),
                    path: path.to_string(),
                    force,
                })),
            })
            .take_while(|r| {
                std::future::ready(!matches!(
                    r,
                    Ok(ServerMessage {
                        msg: Some(server_message::Msg::PluginEvent(PluginEvent {
                            event: Some(plugin_event::Event::Done(d))
                        }))
                    }) if d.error.is_empty()
                ))
            })
            .filter_map(|r| {
                std::future::ready(match r {
                    Ok(ServerMessage {
                        msg: Some(server_message::Msg::PluginEvent(e)),
                    }) => e.event.map(Ok),
                    Ok(ServerMessage {
                        msg: Some(server_message::Msg::Error(e)),
                    }) => Some(Err(anyhow::anyhow!(
                        "server error ({}): {}",
                        e.code,
                        e.message
                    ))),
                    Ok(_) => None,
                    Err(e) => Some(Err(e)),
                })
            })
    }

    /// Uninstall a plugin, streaming progress events.
    pub fn uninstall_plugin<'a>(
        &'a mut self,
        plugin: &str,
    ) -> impl Stream<Item = Result<plugin_event::Event>> + Send + 'a {
        self.transport
            .request_stream(ClientMessage {
                msg: Some(client_message::Msg::UninstallPlugin(UninstallPluginMsg {
                    plugin: plugin.to_string(),
                })),
            })
            .take_while(|r| {
                std::future::ready(!matches!(
                    r,
                    Ok(ServerMessage {
                        msg: Some(server_message::Msg::PluginEvent(PluginEvent {
                            event: Some(plugin_event::Event::Done(d))
                        }))
                    }) if d.error.is_empty()
                ))
            })
            .filter_map(|r| {
                std::future::ready(match r {
                    Ok(ServerMessage {
                        msg: Some(server_message::Msg::PluginEvent(e)),
                    }) => e.event.map(Ok),
                    Ok(ServerMessage {
                        msg: Some(server_message::Msg::Error(e)),
                    }) => Some(Err(anyhow::anyhow!(
                        "server error ({}): {}",
                        e.code,
                        e.message
                    ))),
                    Ok(_) => None,
                    Err(e) => Some(Err(e)),
                })
            })
    }
}

/// Send a `ReplyToAsk` to the daemon on a temporary connection.
pub async fn send_reply(
    conn_info: &ConnectionInfo,
    agent: String,
    sender: String,
    content: String,
) -> Result<()> {
    let msg = ClientMessage::from(ReplyToAsk {
        agent,
        sender,
        content,
    });
    match conn_info {
        #[cfg(unix)]
        ConnectionInfo::Uds(path) => {
            let client = transport::uds::CrabtalkClient::new(transport::uds::ClientConfig {
                socket_path: path.clone(),
            });
            let mut conn = client.connect().await?;
            conn.request(msg).await?;
        }
        ConnectionInfo::Tcp(port) => {
            let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, *port));
            let mut conn = transport::tcp::TcpConnection::connect(addr).await?;
            conn.request(msg).await?;
        }
    }
    Ok(())
}
