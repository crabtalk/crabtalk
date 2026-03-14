//! Client trait — transport primitives plus typed provided methods.

use crate::protocol::message::{
    ClientMessage, ConfigMsg, DownloadEvent, ErrorMsg, GetConfig, HubMsg, MemoryOp, MemoryQueryMsg,
    Ping, SendMsg, SendResponse, ServerMessage, SetConfigMsg, StreamEvent, StreamMsg,
    SubscribeDownloads, SubscribeTasks, client_message, download_event, memory_result,
    server_message, stream_event, task_event,
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
        req: SendMsg,
    ) -> impl std::future::Future<Output = Result<SendResponse>> + Send {
        async move { SendResponse::try_from(self.request(req.into()).await?) }
    }

    /// Send a message to an agent and receive a streamed response.
    fn stream(
        &mut self,
        req: StreamMsg,
    ) -> impl Stream<Item = Result<stream_event::Event>> + Send + '_ {
        self.request_stream(req.into())
            .take_while(|r| {
                std::future::ready(!matches!(
                    r,
                    Ok(ServerMessage {
                        msg: Some(server_message::Msg::Stream(StreamEvent {
                            event: Some(stream_event::Event::End(_))
                        }))
                    })
                ))
            })
            .map(|r| r.and_then(stream_event::Event::try_from))
    }

    /// Install or uninstall a hub package, streaming download events.
    fn hub(
        &mut self,
        req: HubMsg,
    ) -> impl Stream<Item = Result<download_event::Event>> + Send + '_ {
        self.request_stream(ClientMessage {
            msg: Some(client_message::Msg::Hub(req)),
        })
        .take_while(|r| {
            std::future::ready(!matches!(
                r,
                Ok(ServerMessage {
                    msg: Some(server_message::Msg::Download(DownloadEvent {
                        event: Some(download_event::Event::Completed(_))
                    }))
                })
            ))
        })
        .map(|r| r.and_then(download_event::Event::try_from))
    }

    /// Ping the server (keepalive).
    fn ping(&mut self) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::Ping(Ping {})),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
                    anyhow::bail!("server error ({code}): {message}")
                }
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    /// Subscribe to task lifecycle events.
    ///
    /// Streams `task_event::Event`s indefinitely until the connection closes.
    fn subscribe_tasks(&mut self) -> impl Stream<Item = Result<task_event::Event>> + Send + '_ {
        self.request_stream(ClientMessage {
            msg: Some(client_message::Msg::SubscribeTasks(SubscribeTasks {})),
        })
        .map(|r| r.and_then(task_event::Event::try_from))
    }

    /// Subscribe to download lifecycle events.
    ///
    /// Streams `download_event::Event`s indefinitely until the connection closes.
    fn subscribe_downloads(
        &mut self,
    ) -> impl Stream<Item = Result<download_event::Event>> + Send + '_ {
        self.request_stream(ClientMessage {
            msg: Some(client_message::Msg::SubscribeDownloads(
                SubscribeDownloads {},
            )),
        })
        .map(|r| r.and_then(download_event::Event::try_from))
    }

    /// Get the full daemon config as JSON.
    fn get_config(&mut self) -> impl std::future::Future<Output = Result<String>> + Send {
        async move {
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::GetConfig(GetConfig {})),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::Config(ConfigMsg { config })),
                } => Ok(config),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
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
            match self
                .request(ClientMessage {
                    msg: Some(client_message::Msg::SetConfig(SetConfigMsg { config })),
                })
                .await?
            {
                ServerMessage {
                    msg: Some(server_message::Msg::Pong(_)),
                } => Ok(()),
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
                } => {
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
    ) -> impl std::future::Future<Output = Result<memory_result::Result>> + Send {
        async move {
            memory_result::Result::try_from(
                self.request(ClientMessage {
                    msg: Some(client_message::Msg::MemoryQuery(MemoryQueryMsg {
                        query: Some(query),
                    })),
                })
                .await?,
            )
        }
    }
}
