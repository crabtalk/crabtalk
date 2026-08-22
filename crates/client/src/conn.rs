//! Connection bootstrap on top of the [`proto::api::Client`] trait.
//!
//! The trait defines every protocol RPC; transport connections (UDS, TCP, mem)
//! implement it. This module adds:
//!
//! - [`ConnectionInfo`] — a cloneable handle that knows how to (re)connect. It
//!   is told where the daemon is; finding that out is the caller's, since only
//!   a process that knows its install root can answer it.
//! - Typed one-shot sugars on `ConnectionInfo` (`stream`, `reply_to_ask`,
//!   `kill_conversation`, `subscribe_events`) that adapter apps reach for
//!   instead of building `ClientMessage` envelopes by hand.
//!
//! Streaming sugar that maps events onto UI-friendly chunks lives in
//! [`crate::stream`].

use anyhow::Result;
use futures_util::StreamExt;
use proto::api::Client as _;
use proto::{AgentEventMsg, ClientMessage, StreamEvent, StreamMsg, server_message, stream_event};
use std::net::{Ipv4Addr, SocketAddr};
#[cfg(unix)]
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

pub use transport::Transport;

/// How to (re)connect to the daemon.
#[derive(Clone)]
pub enum ConnectionInfo {
    #[cfg(unix)]
    Uds(PathBuf),
    Tcp(u16),
}

impl std::fmt::Display for ConnectionInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(unix)]
            Self::Uds(path) => write!(f, "{}", path.display()),
            Self::Tcp(port) => write!(f, "tcp://127.0.0.1:{port}"),
        }
    }
}

impl ConnectionInfo {
    /// Open a fresh connection, send `req`, and return a receiver of stream
    /// events. The connection closes when the daemon emits `StreamEnd`, when
    /// the server sends an error, or when the receiver is dropped.
    ///
    /// `End` is delivered to the receiver before the channel closes so callers
    /// can observe the terminal usage / error fields if they care.
    pub fn stream(&self, req: StreamMsg) -> mpsc::UnboundedReceiver<Result<stream_event::Event>> {
        let info = self.clone();
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut transport = match connect_from(&info).await {
                Ok(t) => t,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };
            let mut stream = std::pin::pin!(transport.request_stream(ClientMessage::from(req)));
            while let Some(result) = stream.next().await {
                let server_msg = match result {
                    Ok(m) => m,
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        return;
                    }
                };
                match server_msg.msg {
                    Some(server_message::Msg::Stream(StreamEvent { event: Some(ev) })) => {
                        let is_end = matches!(ev, stream_event::Event::End(_));
                        if tx.send(Ok(ev)).is_err() || is_end {
                            return;
                        }
                    }
                    Some(server_message::Msg::Error(e)) => {
                        let _ = tx.send(Err(anyhow::anyhow!(
                            "server error ({}): {}",
                            e.code,
                            e.message
                        )));
                        return;
                    }
                    _ => {}
                }
            }
        });
        rx
    }

    /// Run an anonymous, unpersisted agent turn and return a receiver of
    /// stream events. No conversation is created and nothing reaches
    /// session storage; events are also broadcast (tagged `ephemeral`)
    /// under `correlation_id` so observers can group them. Client
    /// round-trip tools are unsupported on this path — only the agent's
    /// own daemon-side tools run.
    pub fn ephemeral_stream(
        &self,
        agent: String,
        content: String,
        correlation_id: u64,
        tool_choice: Option<String>,
    ) -> mpsc::UnboundedReceiver<Result<stream_event::Event>> {
        self.stream(StreamMsg {
            agent,
            content,
            ephemeral: true,
            correlation_id: Some(correlation_id),
            tool_choice,
            ..Default::default()
        })
    }

    /// Open a fresh connection, kill the active conversation for
    /// `session_handle`, and close. Returns `true` if it existed.
    pub async fn kill_conversation(&self, session_handle: String) -> Result<bool> {
        let mut t = connect_from(self).await?;
        t.kill_conversation(session_handle).await
    }

    /// Open a fresh connection, subscribe to all agent events, and forward
    /// them onto an unbounded channel. The channel closes when the daemon
    /// drops the connection or the receiver is dropped.
    pub fn subscribe_events(&self) -> mpsc::UnboundedReceiver<Result<AgentEventMsg>> {
        let info = self.clone();
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut transport = match connect_from(&info).await {
                Ok(t) => t,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };
            let stream = transport.subscribe_events();
            tokio::pin!(stream);
            while let Some(result) = stream.next().await {
                if tx.send(result).is_err() {
                    break;
                }
            }
        });
        rx
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
