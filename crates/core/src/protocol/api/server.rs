//! Server trait — one async method per protocol operation.

use crate::protocol::message::{
    DownloadEvent, HubAction, MemoryOp, MemoryResult, SendRequest, SendResponse, StreamEvent,
    StreamRequest, TaskEvent,
    client::ClientMessage,
    server::{DownloadInfo, ServerMessage, SessionInfo, TaskInfo},
};
use anyhow::Result;
use futures_core::Stream;
use futures_util::StreamExt;

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
    fn send(
        &self,
        req: SendRequest,
    ) -> impl std::future::Future<Output = Result<SendResponse>> + Send;

    /// Handle `Stream` — run agent and stream response events.
    fn stream(&self, req: StreamRequest) -> impl Stream<Item = Result<StreamEvent>> + Send;

    /// Handle `Ping` — keepalive.
    fn ping(&self) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Handle `Hub` — install or uninstall a hub package.
    fn hub(
        &self,
        package: compact_str::CompactString,
        action: HubAction,
    ) -> impl Stream<Item = Result<DownloadEvent>> + Send;

    /// Handle `Sessions` — list active sessions.
    fn list_sessions(&self) -> impl std::future::Future<Output = Result<Vec<SessionInfo>>> + Send;

    /// Handle `Kill` — close a session by ID.
    fn kill_session(&self, session: u64) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Handle `Tasks` — list tasks in the task registry.
    fn list_tasks(&self) -> impl std::future::Future<Output = Result<Vec<TaskInfo>>> + Send;

    /// Handle `KillTask` — cancel a task by ID.
    fn kill_task(&self, task_id: u64) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Handle `Approve` — approve a blocked task's inbox item.
    fn approve_task(
        &self,
        task_id: u64,
        response: String,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Handle `Evaluate` — decide whether the agent should respond (DD#39).
    fn evaluate(&self, req: SendRequest) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Handle `SubscribeTasks` — stream task lifecycle events.
    fn subscribe_tasks(&self) -> impl Stream<Item = Result<TaskEvent>> + Send;

    /// Handle `Downloads` — list downloads in the registry.
    fn list_downloads(&self)
    -> impl std::future::Future<Output = Result<Vec<DownloadInfo>>> + Send;

    /// Handle `SubscribeDownloads` — stream download lifecycle events.
    fn subscribe_downloads(&self) -> impl Stream<Item = Result<DownloadEvent>> + Send;

    /// Handle `GetConfig` — return the full daemon config as JSON.
    fn get_config(&self) -> impl std::future::Future<Output = Result<String>> + Send;

    /// Handle `SetConfig` — replace the daemon config from JSON.
    fn set_config(&self, config: String) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Handle `MemoryQuery` — query the memory graph.
    fn memory_query(
        &self,
        query: MemoryOp,
    ) -> impl std::future::Future<Output = Result<MemoryResult>> + Send;

    /// Dispatch a `ClientMessage` to the appropriate handler method.
    ///
    /// Returns a stream of `ServerMessage`s. Request-response operations
    /// yield exactly one message; streaming operations yield many.
    fn dispatch(&self, msg: ClientMessage) -> impl Stream<Item = ServerMessage> + Send + '_ {
        async_stream::stream! {
            match msg {
                ClientMessage::Send { agent, content, session, sender } => {
                    yield result_to_msg(self.send(SendRequest { agent, content, session, sender }).await);
                }
                ClientMessage::Stream { agent, content, session, sender } => {
                    let s = self.stream(StreamRequest { agent, content, session, sender });
                    tokio::pin!(s);
                    while let Some(result) = s.next().await {
                        yield result_to_msg(result);
                    }
                }
                ClientMessage::Ping => {
                    yield match self.ping().await {
                        Ok(()) => ServerMessage::Pong,
                        Err(e) => ServerMessage::Error {
                            code: 500,
                            message: e.to_string(),
                        },
                    };
                }
                ClientMessage::Hub { package, action } => {
                    let s = self.hub(package, action);
                    tokio::pin!(s);
                    while let Some(result) = s.next().await {
                        yield result_to_msg(result);
                    }
                }
                ClientMessage::Sessions => {
                    yield match self.list_sessions().await {
                        Ok(sessions) => ServerMessage::Sessions(sessions),
                        Err(e) => ServerMessage::Error {
                            code: 500,
                            message: e.to_string(),
                        },
                    };
                }
                ClientMessage::Kill { session } => {
                    yield match self.kill_session(session).await {
                        Ok(true) => ServerMessage::Pong,
                        Ok(false) => ServerMessage::Error {
                            code: 404,
                            message: format!("session {session} not found"),
                        },
                        Err(e) => ServerMessage::Error {
                            code: 500,
                            message: e.to_string(),
                        },
                    };
                }
                ClientMessage::Tasks => {
                    yield match self.list_tasks().await {
                        Ok(tasks) => ServerMessage::Tasks(tasks),
                        Err(e) => ServerMessage::Error {
                            code: 500,
                            message: e.to_string(),
                        },
                    };
                }
                ClientMessage::KillTask { task_id } => {
                    yield match self.kill_task(task_id).await {
                        Ok(true) => ServerMessage::Pong,
                        Ok(false) => ServerMessage::Error {
                            code: 404,
                            message: format!("task {task_id} not found"),
                        },
                        Err(e) => ServerMessage::Error {
                            code: 500,
                            message: e.to_string(),
                        },
                    };
                }
                ClientMessage::Approve { task_id, response } => {
                    yield match self.approve_task(task_id, response).await {
                        Ok(true) => ServerMessage::Pong,
                        Ok(false) => ServerMessage::Error {
                            code: 404,
                            message: format!("task {task_id} not found or not blocked"),
                        },
                        Err(e) => ServerMessage::Error {
                            code: 500,
                            message: e.to_string(),
                        },
                    };
                }
                ClientMessage::Evaluate { agent, content, session, sender } => {
                    yield match self.evaluate(SendRequest { agent, content, session, sender }).await {
                        Ok(respond) => ServerMessage::Evaluation { respond },
                        Err(e) => ServerMessage::Error {
                            code: 500,
                            message: e.to_string(),
                        },
                    };
                }
                ClientMessage::SubscribeTasks => {
                    let s = self.subscribe_tasks();
                    tokio::pin!(s);
                    while let Some(result) = s.next().await {
                        yield result_to_msg(result);
                    }
                }
                ClientMessage::Downloads => {
                    yield match self.list_downloads().await {
                        Ok(downloads) => ServerMessage::Downloads(downloads),
                        Err(e) => ServerMessage::Error {
                            code: 500,
                            message: e.to_string(),
                        },
                    };
                }
                ClientMessage::SubscribeDownloads => {
                    let s = self.subscribe_downloads();
                    tokio::pin!(s);
                    while let Some(result) = s.next().await {
                        yield result_to_msg(result);
                    }
                }
                ClientMessage::GetConfig => {
                    yield match self.get_config().await {
                        Ok(config) => ServerMessage::Config { config },
                        Err(e) => ServerMessage::Error {
                            code: 500,
                            message: e.to_string(),
                        },
                    };
                }
                ClientMessage::SetConfig { config } => {
                    yield match self.set_config(config).await {
                        Ok(()) => ServerMessage::Pong,
                        Err(e) => ServerMessage::Error {
                            code: 500,
                            message: e.to_string(),
                        },
                    };
                }
                ClientMessage::MemoryQuery { query } => {
                    yield match self.memory_query(query).await {
                        Ok(result) => ServerMessage::Memory(result),
                        Err(e) => ServerMessage::Error {
                            code: 500,
                            message: e.to_string(),
                        },
                    };
                }
            }
        }
    }
}

/// Convert a typed `Result` into a `ServerMessage`.
fn result_to_msg<T: Into<ServerMessage>>(result: Result<T>) -> ServerMessage {
    match result {
        Ok(resp) => resp.into(),
        Err(e) => ServerMessage::Error {
            code: 500,
            message: e.to_string(),
        },
    }
}
