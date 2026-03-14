//! Client trait — transport primitives plus typed provided methods.

use crate::protocol::message::{
    DownloadEvent, DownloadRequest, HubRequest, MemoryOp, MemoryResult, SendRequest, SendResponse,
    StreamEvent, StreamRequest, TaskEvent, client::ClientMessage, server::ServerMessage,
};
use anyhow::Result;
use futures_core::Stream;
use futures_util::StreamExt;

/// Client-side protocol interface.
///
/// Implementors provide two transport primitives — [`request`](Client::request)
/// for request-response and [`request_stream`](Client::request_stream) for
/// streaming operations. All typed methods are provided defaults that delegate
/// to these primitives.
pub trait Client: Send {
    /// Send a `ClientMessage` and receive a single `ServerMessage`.
    fn request(
        &mut self,
        msg: ClientMessage,
    ) -> impl std::future::Future<Output = Result<ServerMessage>> + Send;

    /// Send a `ClientMessage` and receive a stream of `ServerMessage`s.
    ///
    /// This is a raw transport primitive — the stream reads indefinitely.
    /// Callers must detect the terminal sentinel (e.g. `StreamEnd`,
    /// `DownloadEnd`) and stop consuming. The typed streaming methods
    /// handle this automatically.
    fn request_stream(
        &mut self,
        msg: ClientMessage,
    ) -> impl Stream<Item = Result<ServerMessage>> + Send + '_;

    /// Send a message to an agent and receive a complete response.
    fn send(
        &mut self,
        req: SendRequest,
    ) -> impl std::future::Future<Output = Result<SendResponse>> + Send {
        async move { SendResponse::try_from(self.request(req.into()).await?) }
    }

    /// Send a message to an agent and receive a streamed response.
    fn stream(
        &mut self,
        req: StreamRequest,
    ) -> impl Stream<Item = Result<StreamEvent>> + Send + '_ {
        self.request_stream(req.into())
            .take_while(|r| {
                std::future::ready(!matches!(
                    r,
                    Ok(ServerMessage::Stream(StreamEvent::End { .. }))
                ))
            })
            .map(|r| r.and_then(StreamEvent::try_from))
    }

    /// Download a model's files with progress reporting.
    fn download(
        &mut self,
        req: DownloadRequest,
    ) -> impl Stream<Item = Result<DownloadEvent>> + Send + '_ {
        self.request_stream(req.into())
            .take_while(|r| {
                std::future::ready(!matches!(
                    r,
                    Ok(ServerMessage::Download(DownloadEvent::Completed { .. }))
                ))
            })
            .map(|r| r.and_then(DownloadEvent::try_from))
    }

    /// Install or uninstall a hub package, streaming download events.
    fn hub(&mut self, req: HubRequest) -> impl Stream<Item = Result<DownloadEvent>> + Send + '_ {
        self.request_stream(req.into())
            .take_while(|r| {
                std::future::ready(!matches!(
                    r,
                    Ok(ServerMessage::Download(DownloadEvent::Completed { .. }))
                ))
            })
            .map(|r| r.and_then(DownloadEvent::try_from))
    }

    /// Ping the server (keepalive).
    fn ping(&mut self) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self.request(ClientMessage::Ping).await? {
                ServerMessage::Pong => Ok(()),
                ServerMessage::Error { code, message } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Subscribe to task lifecycle events.
    ///
    /// Streams `TaskEvent`s indefinitely until the connection closes.
    fn subscribe_tasks(&mut self) -> impl Stream<Item = Result<TaskEvent>> + Send + '_ {
        self.request_stream(ClientMessage::SubscribeTasks)
            .map(|r| r.and_then(TaskEvent::try_from))
    }

    /// Subscribe to download lifecycle events.
    ///
    /// Streams `DownloadEvent`s indefinitely until the connection closes.
    fn subscribe_downloads(&mut self) -> impl Stream<Item = Result<DownloadEvent>> + Send + '_ {
        self.request_stream(ClientMessage::SubscribeDownloads)
            .map(|r| r.and_then(DownloadEvent::try_from))
    }

    /// Get the full daemon config as JSON.
    fn get_config(&mut self) -> impl std::future::Future<Output = Result<String>> + Send {
        async move {
            match self.request(ClientMessage::GetConfig).await? {
                ServerMessage::Config { config } => Ok(config),
                ServerMessage::Error { code, message } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Replace the full daemon config from JSON.
    fn set_config(
        &mut self,
        config: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self.request(ClientMessage::SetConfig { config }).await? {
                ServerMessage::Pong => Ok(()),
                ServerMessage::Error { code, message } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Query the memory graph.
    fn memory_query(
        &mut self,
        query: MemoryOp,
    ) -> impl std::future::Future<Output = Result<MemoryResult>> + Send {
        async move { MemoryResult::try_from(self.request(ClientMessage::MemoryQuery { query }).await?) }
    }
}
