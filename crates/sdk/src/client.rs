//! Client for connecting to the crabtalk daemon.
//!
//! Creates a fresh connection per message to support concurrent sends
//! from platform adapters (Telegram).

use futures_util::StreamExt;
use std::net::{Ipv4Addr, SocketAddr};
use tokio::sync::mpsc;
use transport::tcp::TcpConnection;
use wcore::protocol::{
    api::Client,
    message::{ClientMessage, ServerMessage},
};

/// How the client connects to the daemon.
pub enum NodeTransport {
    /// Connect via Unix domain socket.
    #[cfg(unix)]
    Uds(std::path::PathBuf),
    /// Connect via TCP on localhost.
    Tcp(u16),
}

/// Client that sends `ClientMessage`s to the daemon.
///
/// Each call to [`send`] opens a new connection, sends the message, and
/// returns a receiver that streams back `ServerMessage` responses until
/// the daemon closes the connection.
pub struct NodeClient {
    transport: NodeTransport,
}

impl NodeClient {
    /// Create a client using the platform default transport:
    /// UDS on Unix, TCP (from port file) on Windows.
    pub fn platform_default() -> anyhow::Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self::uds(&wcore::paths::SOCKET_PATH))
        }
        #[cfg(not(unix))]
        {
            let port_str = std::fs::read_to_string(&*wcore::paths::TCP_PORT_FILE)?;
            let port: u16 = port_str.trim().parse()?;
            Ok(Self::tcp(port))
        }
    }

    /// Create a client that connects via TCP on the given port.
    pub fn tcp(port: u16) -> Self {
        Self {
            transport: NodeTransport::Tcp(port),
        }
    }

    /// Create a client that connects via Unix domain socket.
    #[cfg(unix)]
    pub fn uds(socket_path: &std::path::Path) -> Self {
        Self {
            transport: NodeTransport::Uds(socket_path.to_path_buf()),
        }
    }

    /// Send a message to the daemon and return a receiver for streamed replies.
    ///
    /// Opens a fresh connection per call so multiple platform loops can send
    /// concurrently without blocking.
    pub async fn send(&self, msg: ClientMessage) -> mpsc::UnboundedReceiver<ServerMessage> {
        let (tx, rx) = mpsc::unbounded_channel();

        macro_rules! spawn_stream {
            ($conn:expr, $msg:expr, $tx:expr) => {{
                let mut conn = $conn;
                let msg = $msg;
                let tx = $tx;
                tokio::spawn(async move {
                    let mut stream = std::pin::pin!(conn.request_stream(msg));
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
            }};
        }

        match &self.transport {
            #[cfg(unix)]
            NodeTransport::Uds(socket_path) => {
                match transport::uds::CrabtalkClient::new(transport::uds::ClientConfig {
                    socket_path: socket_path.clone(),
                })
                .connect()
                .await
                {
                    Ok(conn) => spawn_stream!(conn, msg, tx),
                    Err(e) => tracing::error!("failed to connect to daemon: {e}"),
                }
            }
            NodeTransport::Tcp(port) => {
                let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, *port));
                match TcpConnection::connect(addr).await {
                    Ok(conn) => spawn_stream!(conn, msg, tx),
                    Err(e) => tracing::error!("failed to connect to daemon: {e}"),
                }
            }
        }
        rx
    }
}
