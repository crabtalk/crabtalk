//! UDS client for connecting to the walrus daemon.
//!
//! Creates a fresh connection per message to support concurrent sends
//! from platform adapters (Telegram).

use futures_util::StreamExt;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use wcore::protocol::{
    api::Client,
    message::{ClientMessage, ServerMessage},
};

/// Client that sends `ClientMessage`s to the daemon over UDS.
///
/// Each call to [`send`] opens a new connection, sends the message, and
/// returns a receiver that streams back `ServerMessage` responses until
/// the daemon closes the connection.
pub struct DaemonClient {
    socket_path: PathBuf,
}

impl DaemonClient {
    pub fn new(socket_path: &Path) -> Self {
        Self {
            socket_path: socket_path.to_path_buf(),
        }
    }

    /// Send a message to the daemon and return a receiver for streamed replies.
    ///
    /// Opens a fresh UDS connection per call (cheap on local sockets) so
    /// multiple platform loops can send concurrently without blocking.
    pub async fn send(&self, msg: ClientMessage) -> mpsc::UnboundedReceiver<ServerMessage> {
        let (tx, rx) = mpsc::unbounded_channel();
        match socket::WalrusClient::new(socket::ClientConfig {
            socket_path: self.socket_path.clone(),
        })
        .connect()
        .await
        {
            Ok(mut conn) => {
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
            }
            Err(e) => {
                tracing::error!("failed to connect to daemon: {e}");
            }
        }
        rx
    }
}
