//! TCP client for connecting to a walrus daemon.

use anyhow::Result;
use futures_core::Stream;
use std::net::SocketAddr;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use wcore::protocol::{
    api::Client,
    codec,
    message::{ClientMessage, ErrorMsg, ServerMessage, server_message},
};

/// Client configuration for connecting to a walrus daemon over TCP.
#[derive(Debug, Clone)]
pub struct TcpClientConfig {
    /// Daemon TCP address.
    pub addr: SocketAddr,
}

/// TCP client for the walrus daemon.
///
/// Holds configuration. Call [`TcpClient::connect`] to establish a connection.
pub struct TcpClient {
    config: TcpClientConfig,
}

impl TcpClient {
    /// Create a new client with the given configuration.
    pub fn new(config: TcpClientConfig) -> Self {
        Self { config }
    }

    /// Access the client configuration.
    pub fn config(&self) -> &TcpClientConfig {
        &self.config
    }

    /// Connect to the daemon and return a [`TcpConnection`].
    pub async fn connect(&self) -> Result<TcpConnection> {
        TcpConnection::connect(self.config.addr).await
    }
}

/// An established TCP connection to a walrus daemon.
///
/// Not Clone — one connection per session. Use [`TcpClient::connect`]
/// to create a connection.
pub struct TcpConnection {
    reader: OwnedReadHalf,
    writer: OwnedWriteHalf,
}

impl TcpConnection {
    /// Connect to a daemon at the given address.
    pub async fn connect(addr: SocketAddr) -> Result<Self> {
        let stream = tokio::net::TcpStream::connect(addr).await?;
        stream.set_nodelay(true)?;
        tracing::debug!("connected to {addr}");
        let (reader, writer) = stream.into_split();
        Ok(Self { reader, writer })
    }
}

impl Client for TcpConnection {
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
