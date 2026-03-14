//! Unix domain socket client for connecting to a walrus daemon.

use anyhow::Result;
use futures_core::Stream;
use std::path::{Path, PathBuf};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use wcore::protocol::{
    api::Client,
    codec,
    message::{ClientMessage, ErrorMsg, ServerMessage, server_message},
};

/// Client configuration for connecting to a walrus daemon.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Daemon Unix domain socket path.
    pub socket_path: PathBuf,
}

/// Unix domain socket client for the walrus daemon.
///
/// Holds configuration. Call [`WalrusClient::connect`] to establish a
/// connection.
pub struct WalrusClient {
    config: ClientConfig,
}

impl WalrusClient {
    /// Create a new client with the given configuration.
    pub fn new(config: ClientConfig) -> Self {
        Self { config }
    }

    /// Access the client configuration.
    pub fn config(&self) -> &ClientConfig {
        &self.config
    }

    /// Set the daemon socket path.
    pub fn socket_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.socket_path = path.into();
        self
    }

    /// Connect to the daemon and return a [`Connection`].
    pub async fn connect(&self) -> Result<Connection> {
        Connection::connect(&self.config.socket_path).await
    }
}

/// An established Unix domain socket connection to a walrus daemon.
///
/// Not Clone — one connection per session. Use [`WalrusClient::connect`]
/// to create a connection.
pub struct Connection {
    reader: OwnedReadHalf,
    writer: OwnedWriteHalf,
}

impl Connection {
    /// Connect to a daemon at the given socket path.
    pub async fn connect(socket_path: &Path) -> Result<Self> {
        let stream = tokio::net::UnixStream::connect(socket_path).await?;
        tracing::debug!("connected to {}", socket_path.display());
        let (reader, writer) = stream.into_split();
        Ok(Self { reader, writer })
    }
}

impl Client for Connection {
    async fn request(&mut self, msg: ClientMessage) -> Result<ServerMessage> {
        codec::write_message(&mut self.writer, &msg).await?;
        Ok(codec::read_message(&mut self.reader).await?)
    }

    fn request_stream(
        &mut self,
        msg: ClientMessage,
    ) -> impl Stream<Item = Result<ServerMessage>> + Send + '_ {
        async_stream::try_stream! {
            codec::write_message(&mut self.writer, &msg).await?;

            loop {
                let server_msg: ServerMessage = codec::read_message(&mut self.reader).await?;

                match &server_msg {
                    ServerMessage { msg: Some(server_message::Msg::Error(ErrorMsg { code, message })) } => {
                        Err(anyhow::anyhow!("server error ({code}): {message}"))?;
                    }
                    _ => yield server_msg,
                }
            }
        }
    }
}
