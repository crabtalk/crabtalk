//! Daemon client — connection bootstrap on top of the [`api::Client`] trait.
//!
//! The trait in `wcore::protocol::api` defines every protocol RPC. Transport
//! connections (UDS, TCP) implement it. This module exists only to:
//!   - bootstrap a connection ([`connect_uds`], [`connect_tcp`], [`connect_from`]),
//!   - hold reconnection metadata ([`ConnectionInfo`]),
//!   - and offer an [`OutputChunk`] adapter over [`api::Client::stream`] for
//!     UI consumers (TUI, gateway adapters).

use anyhow::Result;
use futures_core::Stream;
use futures_util::StreamExt;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use tokio::sync::mpsc;
use wcore::protocol::{api::Client as _, message::*};

pub use transport::Transport;

/// How to (re)connect to the daemon.
#[derive(Clone)]
pub enum ConnectionInfo {
    #[cfg(unix)]
    Uds(PathBuf),
    Tcp(u16),
}

impl ConnectionInfo {
    /// Resolve the platform default: UDS on Unix, TCP (from port file) on Windows.
    pub fn platform_default() -> Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self::Uds(wcore::paths::SOCKET_PATH.to_path_buf()))
        }
        #[cfg(not(unix))]
        {
            let port_str = std::fs::read_to_string(&*wcore::paths::TCP_PORT_FILE)?;
            let port: u16 = port_str.trim().parse()?;
            Ok(Self::Tcp(port))
        }
    }

    /// Open a fresh connection, send `msg`, and return a receiver of server
    /// replies. The connection closes when the daemon ends the stream or the
    /// receiver is dropped.
    ///
    /// Use this for short-lived consumers (cron fires, gateway stream-per-chat)
    /// where a long-lived [`Transport`] would just serialize concurrent senders.
    pub async fn send(&self, msg: ClientMessage) -> Result<mpsc::UnboundedReceiver<ServerMessage>> {
        let mut transport = connect_from(self).await?;
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut stream = std::pin::pin!(transport.request_stream(msg));
            while let Some(result) = stream.next().await {
                match result {
                    Ok(server_msg) => {
                        if tx.send(server_msg).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("daemon stream error: {e}");
                        break;
                    }
                }
            }
        });
        Ok(rx)
    }
}

/// Connect to the daemon over a Unix domain socket.
#[cfg(unix)]
pub async fn connect_uds(socket_path: &Path) -> Result<Transport> {
    let config = transport::uds::ClientConfig {
        socket_path: socket_path.to_path_buf(),
    };
    let connection = transport::uds::CrabtalkClient::new(config)
        .connect()
        .await?;
    Ok(Transport::Uds(connection))
}

/// Connect to the daemon over TCP on localhost.
pub async fn connect_tcp(port: u16) -> Result<Transport> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let connection = transport::tcp::TcpConnection::connect(addr).await?;
    Ok(Transport::Tcp(connection))
}

/// Open a fresh connection from previously-captured [`ConnectionInfo`].
pub async fn connect_from(info: &ConnectionInfo) -> Result<Transport> {
    match info {
        #[cfg(unix)]
        ConnectionInfo::Uds(path) => connect_uds(path).await,
        ConnectionInfo::Tcp(port) => connect_tcp(*port).await,
    }
}

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
    /// Tool result returned (call_id, output).
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

/// Run a [`StreamMsg`] on `transport` and translate `stream_event::Event`
/// into UI-friendly [`OutputChunk`]s. Filters telemetry-only events
/// (`Start`, `End`, `ContextUsage`, `UserSteered`).
pub fn stream_chunks<'a>(
    transport: &'a mut Transport,
    req: StreamMsg,
) -> impl Stream<Item = Result<OutputChunk>> + Send + 'a {
    let agent = req.agent.clone();
    let sender = req.sender.clone().unwrap_or_default();
    transport
        .stream(req)
        .scan((agent, sender), |state, result| {
            let chunk = match result {
                Ok(stream_event::Event::Chunk(c)) => Some(Ok(OutputChunk::Text(c.content))),
                Ok(stream_event::Event::Thinking(t)) => Some(Ok(OutputChunk::Thinking(t.content))),
                Ok(stream_event::Event::ToolStart(ts)) => {
                    let calls = ts
                        .calls
                        .into_iter()
                        .map(|c| (c.name, c.arguments))
                        .collect();
                    Some(Ok(OutputChunk::ToolStart(calls)))
                }
                Ok(stream_event::Event::ToolResult(tr)) => {
                    Some(Ok(OutputChunk::ToolResult(tr.call_id, tr.output)))
                }
                Ok(stream_event::Event::ToolsComplete(_)) => Some(Ok(OutputChunk::ToolDone(true))),
                Ok(stream_event::Event::AskUser(ask)) => Some(Ok(OutputChunk::AskUser {
                    questions: ask.questions,
                    agent: state.0.clone(),
                    sender: state.1.clone(),
                })),
                Ok(stream_event::Event::TextStart(_)) => Some(Ok(OutputChunk::TextStart)),
                Ok(stream_event::Event::TextEnd(_)) => Some(Ok(OutputChunk::TextEnd)),
                Ok(stream_event::Event::ThinkingStart(_)) => Some(Ok(OutputChunk::ThinkingStart)),
                Ok(stream_event::Event::ThinkingEnd(_)) => Some(Ok(OutputChunk::ThinkingEnd)),
                Ok(stream_event::Event::Start(_))
                | Ok(stream_event::Event::UserSteered(_))
                | Ok(stream_event::Event::ContextUsage(_))
                | Ok(stream_event::Event::End(_)) => None,
                Err(e) => Some(Err(e)),
            };
            std::future::ready(Some(chunk))
        })
        .filter_map(std::future::ready)
}
