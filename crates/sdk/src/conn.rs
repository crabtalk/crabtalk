//! Connection bootstrap on top of the [`wcore::protocol::api::Client`] trait.
//!
//! The trait defines every protocol RPC; transport connections (UDS, TCP, mem)
//! implement it. This module adds:
//!
//! - [`ConnectionInfo`] — a cloneable handle that knows how to (re)connect.
//! - Typed one-shot sugars on `ConnectionInfo` (`stream`, `reply_to_ask`,
//!   `kill_conversation`, `subscribe_events`) that adapter apps reach for
//!   instead of building `ClientMessage` envelopes by hand.
//!
//! Streaming sugar that maps events onto UI-friendly chunks lives in
//! [`crate::stream`].

use anyhow::Result;
use futures_util::StreamExt;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use tokio::sync::mpsc;
use wcore::protocol::{
    api::Client as _,
    message::{AgentEventMsg, ClientMessage, StreamEvent, StreamMsg, server_message, stream_event},
};

pub use transport::{
    Transport,
    mem::{MemConnection, connect as connect_mem},
};

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

    /// Open a fresh connection, deliver a client-side tool result for the
    /// pending forwarded call keyed by `(conversation_id, call_id)`, and
    /// close.
    pub async fn reply_to_tool(
        &self,
        conversation_id: u64,
        call_id: String,
        output: String,
        is_error: bool,
    ) -> Result<()> {
        let mut t = connect_from(self).await?;
        t.reply_to_tool(conversation_id, call_id, output, is_error)
            .await
    }

    /// Open a fresh connection, kill the active conversation for
    /// `(agent, sender)`, and close. Returns `true` if it existed.
    pub async fn kill_conversation(&self, agent: String, sender: String) -> Result<bool> {
        let mut t = connect_from(self).await?;
        t.kill_conversation(agent, sender).await
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
