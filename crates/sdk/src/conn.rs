//! Connection bootstrap on top of the [`wcore::protocol::api::Client`] trait.
//!
//! The trait defines every protocol RPC; transport connections (UDS, TCP, mem)
//! implement it. This module only handles connection construction and
//! reconnection metadata — streaming sugar lives in [`crate::stream`].

use anyhow::Result;
use futures_util::StreamExt;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use tokio::sync::mpsc;
use wcore::protocol::{
    api::Client as _,
    message::{ClientMessage, ServerMessage},
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

    /// Open a fresh connection, send `msg`, and return a receiver of server
    /// replies. The connection closes when the daemon ends the stream or the
    /// receiver is dropped.
    ///
    /// Use this for short-lived consumers (cron fires, chat-platform
    /// stream-per-chat) where a long-lived [`Transport`] would just serialize
    /// concurrent senders.
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
