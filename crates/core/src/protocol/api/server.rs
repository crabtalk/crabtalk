//! Server trait — one async method per protocol operation.

use crate::protocol::message::{
    AgentEventMsg, ClientMessage, ConfigMsg, ErrorMsg, Pong, SendMsg, SendResponse, ServerMessage,
    SessionInfo, SessionList, StreamEvent, StreamMsg, client_message, server_message,
};
use anyhow::Result;
use futures_core::Stream;
use futures_util::StreamExt;

/// Construct an error `ServerMessage`.
fn server_error(code: u32, message: String) -> ServerMessage {
    ServerMessage {
        msg: Some(server_message::Msg::Error(ErrorMsg { code, message })),
    }
}

/// Construct a pong `ServerMessage`.
fn server_pong() -> ServerMessage {
    ServerMessage {
        msg: Some(server_message::Msg::Pong(Pong {})),
    }
}

/// Convert a typed `Result` into a `ServerMessage`.
fn result_to_msg<T: Into<ServerMessage>>(result: Result<T>) -> ServerMessage {
    match result {
        Ok(resp) => resp.into(),
        Err(e) => server_error(500, e.to_string()),
    }
}

/// Server-side protocol handler.
///
/// Each method corresponds to one `ClientMessage` variant. Implementations
/// receive typed request structs and return typed responses — no enum matching
/// required. Streaming operations return `impl Stream`.
///
/// The provided [`dispatch`](Server::dispatch) method routes a raw
/// `ClientMessage` to the appropriate handler, returning a stream of
/// `ServerMessage`s.
pub trait Server: Sync {
    /// Handle `Send` — run agent and return complete response.
    fn send(&self, req: SendMsg) -> impl std::future::Future<Output = Result<SendResponse>> + Send;

    /// Handle `Stream` — run agent and stream response events.
    fn stream(&self, req: StreamMsg) -> impl Stream<Item = Result<StreamEvent>> + Send;

    /// Handle `Ping` — keepalive.
    fn ping(&self) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Handle `Sessions` — list active sessions.
    fn list_sessions(&self) -> impl std::future::Future<Output = Result<Vec<SessionInfo>>> + Send;

    /// Handle `Kill` — close a session by ID.
    fn kill_session(&self, session: u64) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Handle `SubscribeEvents` — stream agent events.
    fn subscribe_events(&self) -> impl Stream<Item = Result<AgentEventMsg>> + Send;

    /// Handle `GetConfig` — return the full daemon config as JSON.
    fn get_config(&self) -> impl std::future::Future<Output = Result<String>> + Send;

    /// Handle `SetConfig` — replace the daemon config from JSON.
    fn set_config(&self, config: String) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Handle `Reload` — hot-reload runtime from disk.
    fn reload(&self) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Handle `ReplyToAsk` — deliver a user reply to a pending `ask_user` tool call.
    fn reply_to_ask(
        &self,
        session: u64,
        content: String,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Dispatch a `ClientMessage` to the appropriate handler method.
    ///
    /// Returns a stream of `ServerMessage`s. Request-response operations
    /// yield exactly one message; streaming operations yield many.
    fn dispatch(&self, msg: ClientMessage) -> impl Stream<Item = ServerMessage> + Send + '_ {
        async_stream::stream! {
            let Some(inner) = msg.msg else {
                yield server_error(400, "empty client message".to_string());
                return;
            };

            match inner {
                client_message::Msg::Send(send_msg) => {
                    yield result_to_msg(self.send(send_msg).await);
                }
                client_message::Msg::Stream(stream_msg) => {
                    let s = self.stream(stream_msg);
                    tokio::pin!(s);
                    while let Some(result) = s.next().await {
                        yield result_to_msg(result);
                    }
                }
                client_message::Msg::Ping(_) => {
                    yield match self.ping().await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::Sessions(_) => {
                    yield match self.list_sessions().await {
                        Ok(sessions) => ServerMessage {
                            msg: Some(server_message::Msg::Sessions(SessionList { sessions })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::Kill(kill_msg) => {
                    yield match self.kill_session(kill_msg.session).await {
                        Ok(true) => server_pong(),
                        Ok(false) => server_error(
                            404,
                            format!("session {} not found", kill_msg.session),
                        ),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::GetConfig(_) => {
                    yield match self.get_config().await {
                        Ok(config) => ServerMessage {
                            msg: Some(server_message::Msg::Config(ConfigMsg { config })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::SetConfig(set_config_msg) => {
                    yield match self.set_config(set_config_msg.config).await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::SubscribeEvents(_) => {
                    let s = self.subscribe_events();
                    tokio::pin!(s);
                    while let Some(result) = s.next().await {
                        yield result_to_msg(result);
                    }
                }
                client_message::Msg::Reload(_) => {
                    yield match self.reload().await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::ReplyToAsk(msg) => {
                    yield match self.reply_to_ask(msg.session, msg.content).await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(404, e.to_string()),
                    };
                }
            }
        }
    }
}
