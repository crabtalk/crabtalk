//! Server trait — one async method per protocol operation.

use crate::protocol::message::{
    ClientMessage, ConfigMsg, DownloadEvent, DownloadInfo, DownloadList, ErrorMsg, EvaluationMsg,
    HubAction, MemoryOp, MemoryResult, Pong, SendMsg, SendResponse, ServerMessage, SessionInfo,
    SessionList, StreamEvent, StreamMsg, TaskEvent, TaskInfo, TaskList, client_message,
    server_message,
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

    /// Handle `Hub` — install or uninstall a hub package.
    fn hub(
        &self,
        package: String,
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
    fn evaluate(&self, req: SendMsg) -> impl std::future::Future<Output = Result<bool>> + Send;

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
                client_message::Msg::Hub(hub_msg) => {
                    let action = hub_msg.action();
                    let s = self.hub(hub_msg.package, action);
                    tokio::pin!(s);
                    while let Some(result) = s.next().await {
                        yield result_to_msg(result);
                    }
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
                client_message::Msg::Tasks(_) => {
                    yield match self.list_tasks().await {
                        Ok(tasks) => ServerMessage {
                            msg: Some(server_message::Msg::Tasks(TaskList { tasks })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::KillTask(kill_task_msg) => {
                    yield match self.kill_task(kill_task_msg.task_id).await {
                        Ok(true) => server_pong(),
                        Ok(false) => server_error(
                            404,
                            format!("task {} not found", kill_task_msg.task_id),
                        ),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::Approve(approve_msg) => {
                    yield match self.approve_task(approve_msg.task_id, approve_msg.response).await {
                        Ok(true) => server_pong(),
                        Ok(false) => server_error(
                            404,
                            format!("task {} not found or not blocked", approve_msg.task_id),
                        ),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::Evaluate(eval_msg) => {
                    let req = SendMsg {
                        agent: eval_msg.agent,
                        content: eval_msg.content,
                        session: eval_msg.session,
                        sender: eval_msg.sender,
                    };
                    yield match self.evaluate(req).await {
                        Ok(respond) => ServerMessage {
                            msg: Some(server_message::Msg::Evaluation(EvaluationMsg { respond })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::SubscribeTasks(_) => {
                    let s = self.subscribe_tasks();
                    tokio::pin!(s);
                    while let Some(result) = s.next().await {
                        yield result_to_msg(result);
                    }
                }
                client_message::Msg::Downloads(_) => {
                    yield match self.list_downloads().await {
                        Ok(downloads) => ServerMessage {
                            msg: Some(server_message::Msg::Downloads(DownloadList { downloads })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::SubscribeDownloads(_) => {
                    let s = self.subscribe_downloads();
                    tokio::pin!(s);
                    while let Some(result) = s.next().await {
                        yield result_to_msg(result);
                    }
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
                client_message::Msg::MemoryQuery(memory_query_msg) => {
                    let Some(query) = memory_query_msg.query else {
                        yield server_error(400, "empty memory query".to_string());
                        return;
                    };
                    yield match self.memory_query(query).await {
                        Ok(result) => ServerMessage {
                            msg: Some(server_message::Msg::Memory(result)),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
            }
        }
    }
}
