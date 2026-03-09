//! Gateway runner — connects to walrusd via Unix domain socket.

use anyhow::Result;
use compact_str::CompactString;
use futures_core::Stream;
use futures_util::StreamExt;
use socket::{ClientConfig, Connection, WalrusClient};
use std::path::Path;
use wcore::protocol::message::{
    DownloadEvent, DownloadRequest, HubAction, HubEvent, HubRequest, SendRequest, StreamEvent,
    StreamRequest,
    client::ClientMessage,
    server::{ServerMessage, SessionInfo},
};

/// Runs agents via a walrusd Unix domain socket connection.
pub struct Runner {
    connection: Connection,
}

impl Runner {
    /// Connect to walrusd.
    pub async fn connect(socket_path: &Path) -> Result<Self> {
        let config = ClientConfig {
            socket_path: socket_path.to_path_buf(),
        };
        let client = WalrusClient::new(config);
        let connection = client.connect().await?;
        Ok(Self { connection })
    }

    /// Send a one-shot message and return the response content.
    pub async fn send(&mut self, agent: &str, content: &str) -> Result<String> {
        use wcore::protocol::api::Client;
        let resp = self
            .connection
            .send(SendRequest {
                agent: CompactString::from(agent),
                content: content.to_string(),
                session: None,
            })
            .await?;
        Ok(resp.content)
    }

    /// Stream a response, yielding content text chunks.
    pub fn stream<'a>(
        &'a mut self,
        agent: &'a str,
        content: &'a str,
    ) -> impl Stream<Item = Result<String>> + Send + 'a {
        use wcore::protocol::api::Client;
        self.connection
            .stream(StreamRequest {
                agent: CompactString::from(agent),
                content: content.to_string(),
                session: None,
            })
            .filter_map(|result| async {
                match result {
                    Ok(StreamEvent::Chunk { content }) => Some(Ok(content)),
                    Ok(StreamEvent::Start { .. }) => None,
                    Ok(StreamEvent::End { .. }) => None,
                    Err(e) => Some(Err(e)),
                }
            })
    }

    /// Send a download request and return a stream of progress events.
    pub fn download_stream(
        &mut self,
        model: &str,
    ) -> impl Stream<Item = Result<DownloadEvent>> + '_ {
        use wcore::protocol::api::Client;
        self.connection.download(DownloadRequest {
            model: CompactString::from(model),
        })
    }

    /// Send a hub install/uninstall request and return a stream of progress events.
    pub fn hub_stream(
        &mut self,
        package: &str,
        action: HubAction,
    ) -> impl Stream<Item = Result<HubEvent>> + '_ {
        use wcore::protocol::api::Client;
        self.connection.hub(HubRequest {
            package: CompactString::from(package),
            action,
        })
    }

    /// List active sessions on the daemon.
    pub async fn list_sessions(&mut self) -> Result<Vec<SessionInfo>> {
        use wcore::protocol::api::Client;
        match self.connection.request(ClientMessage::Sessions).await? {
            ServerMessage::Sessions(sessions) => Ok(sessions),
            ServerMessage::Error { code, message } => {
                anyhow::bail!("server error ({code}): {message}")
            }
            other => anyhow::bail!("unexpected response: {other:?}"),
        }
    }

    /// Kill (close) a session by ID. Returns true if it existed.
    pub async fn kill_session(&mut self, session: u64) -> Result<bool> {
        use wcore::protocol::api::Client;
        match self
            .connection
            .request(ClientMessage::Kill { session })
            .await?
        {
            ServerMessage::Pong => Ok(true),
            ServerMessage::Error { code: 404, .. } => Ok(false),
            ServerMessage::Error { code, message } => {
                anyhow::bail!("server error ({code}): {message}")
            }
            other => anyhow::bail!("unexpected response: {other:?}"),
        }
    }
}
