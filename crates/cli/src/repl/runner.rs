//! Gateway runner — connects to walrusd via Unix domain socket or TCP.

use anyhow::Result;
use compact_str::CompactString;
use futures_core::Stream;
use futures_util::StreamExt;
use socket::{ClientConfig, Connection, WalrusClient};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use tcp::TcpConnection;
use wcore::protocol::{
    api::Client,
    message::{
        DownloadEvent, HubAction, HubRequest, SendRequest, StreamEvent, StreamRequest,
        client::ClientMessage,
        server::{ServerMessage, SessionInfo, TaskInfo},
    },
};

/// A typed chunk from the streaming response.
pub enum OutputChunk {
    /// Regular text content.
    Text(String),
    /// Thinking/reasoning content (displayed dimmed).
    Thinking(String),
    /// Status message (tool calls, etc.).
    Status(String),
}

/// Transport-agnostic connection to walrusd.
enum Transport {
    Uds(Connection),
    Tcp(TcpConnection),
}

impl Transport {
    async fn request(&mut self, msg: ClientMessage) -> Result<ServerMessage> {
        match self {
            Self::Uds(c) => c.request(msg).await,
            Self::Tcp(c) => c.request(msg).await,
        }
    }

    fn request_stream(
        &mut self,
        msg: ClientMessage,
    ) -> impl Stream<Item = Result<ServerMessage>> + Send + '_ {
        async_stream::try_stream! {
            match self {
                Self::Uds(c) => {
                    let s = c.request_stream(msg);
                    tokio::pin!(s);
                    while let Some(item) = s.next().await {
                        yield item?;
                    }
                }
                Self::Tcp(c) => {
                    let s = c.request_stream(msg);
                    tokio::pin!(s);
                    while let Some(item) = s.next().await {
                        yield item?;
                    }
                }
            }
        }
    }
}

/// Runs agents via a walrusd connection (UDS or TCP).
pub struct Runner {
    transport: Transport,
}

impl Runner {
    /// Connect to walrusd via Unix domain socket.
    pub async fn connect(socket_path: &Path) -> Result<Self> {
        let config = ClientConfig {
            socket_path: socket_path.to_path_buf(),
        };
        let client = WalrusClient::new(config);
        let connection = client.connect().await?;
        Ok(Self {
            transport: Transport::Uds(connection),
        })
    }

    /// Connect to walrusd via TCP.
    pub async fn connect_tcp(port: u16) -> Result<Self> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let connection = TcpConnection::connect(addr).await?;
        Ok(Self {
            transport: Transport::Tcp(connection),
        })
    }

    /// Send a one-shot message and return the response content.
    pub async fn send(&mut self, agent: &str, content: &str) -> Result<String> {
        let resp = self
            .transport
            .request(
                SendRequest {
                    agent: CompactString::from(agent),
                    content: content.to_string(),
                    session: None,
                    sender: None,
                }
                .into(),
            )
            .await?;
        let resp = wcore::protocol::message::SendResponse::try_from(resp)?;
        Ok(resp.content)
    }

    /// Stream a response, yielding typed output chunks.
    pub fn stream<'a>(
        &'a mut self,
        agent: &'a str,
        content: &'a str,
    ) -> impl Stream<Item = Result<OutputChunk>> + Send + 'a {
        self.transport
            .request_stream(
                StreamRequest {
                    agent: CompactString::from(agent),
                    content: content.to_string(),
                    session: None,
                    sender: None,
                }
                .into(),
            )
            .take_while(|r| {
                std::future::ready(!matches!(
                    r,
                    Ok(ServerMessage::Stream(StreamEvent::End { .. }))
                ))
            })
            .filter_map(|result| async {
                match result {
                    Ok(ServerMessage::Stream(StreamEvent::Chunk { content })) => {
                        Some(Ok(OutputChunk::Text(content)))
                    }
                    Ok(ServerMessage::Stream(StreamEvent::Thinking { content })) => {
                        Some(Ok(OutputChunk::Thinking(content)))
                    }
                    Ok(ServerMessage::Stream(StreamEvent::ToolStart { calls })) => {
                        let names: Vec<_> = calls.iter().map(|c| c.name.as_str()).collect();
                        Some(Ok(OutputChunk::Status(format!(
                            "\n[calling {}...]\n",
                            names.join(", ")
                        ))))
                    }
                    Ok(ServerMessage::Stream(StreamEvent::ToolResult { .. })) => None,
                    Ok(ServerMessage::Stream(StreamEvent::ToolsComplete)) => {
                        Some(Ok(OutputChunk::Status("[done]\n".to_string())))
                    }
                    Ok(ServerMessage::Stream(StreamEvent::Start { .. })) => None,
                    Ok(ServerMessage::Stream(StreamEvent::End { .. })) => None,
                    Ok(ServerMessage::Error { code, message }) => {
                        Some(Err(anyhow::anyhow!("server error ({code}): {message}")))
                    }
                    Ok(_) => None,
                    Err(e) => Some(Err(e)),
                }
            })
    }

    /// Send a hub install/uninstall request and return a stream of progress events.
    pub fn hub_stream(
        &mut self,
        package: &str,
        action: HubAction,
    ) -> impl Stream<Item = Result<DownloadEvent>> + '_ {
        self.transport
            .request_stream(
                HubRequest {
                    package: CompactString::from(package),
                    action,
                }
                .into(),
            )
            .take_while(|r| {
                std::future::ready(!matches!(
                    r,
                    Ok(ServerMessage::Download(DownloadEvent::Completed { .. }))
                ))
            })
            .map(|r| r.and_then(DownloadEvent::try_from))
    }

    /// List active sessions on the daemon.
    pub async fn list_sessions(&mut self) -> Result<Vec<SessionInfo>> {
        match self.transport.request(ClientMessage::Sessions).await? {
            ServerMessage::Sessions(sessions) => Ok(sessions),
            ServerMessage::Error { code, message } => {
                anyhow::bail!("server error ({code}): {message}")
            }
            other => anyhow::bail!("unexpected response: {other:?}"),
        }
    }

    /// Kill (close) a session by ID. Returns true if it existed.
    pub async fn kill_session(&mut self, session: u64) -> Result<bool> {
        match self
            .transport
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

    /// List tasks in the task registry.
    pub async fn list_tasks(&mut self) -> Result<Vec<TaskInfo>> {
        match self.transport.request(ClientMessage::Tasks).await? {
            ServerMessage::Tasks(tasks) => Ok(tasks),
            ServerMessage::Error { code, message } => {
                anyhow::bail!("server error ({code}): {message}")
            }
            other => anyhow::bail!("unexpected response: {other:?}"),
        }
    }

    /// Kill (cancel) a task by ID. Returns true if it existed.
    pub async fn kill_task(&mut self, task_id: u64) -> Result<bool> {
        match self
            .transport
            .request(ClientMessage::KillTask { task_id })
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

    /// Approve a blocked task. Returns true if the task was blocked and approved.
    pub async fn approve_task(&mut self, task_id: u64, response: String) -> Result<bool> {
        match self
            .transport
            .request(ClientMessage::Approve { task_id, response })
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
