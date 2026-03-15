//! Server trait — one async method per protocol operation.

use crate::protocol::message::{
    AllSchemasMsg, ClientMessage, ConfigMsg, DownloadEvent, DownloadInfo, DownloadList, ErrorMsg,
    HubAction, Pong, SendMsg, SendResponse, ServerMessage, ServiceInfoMsg, ServiceListMsg,
    ServiceQueryResultMsg, ServiceSchemaMsg, SessionInfo, SessionList, StreamEvent, StreamMsg,
    TaskEvent, TaskInfo, TaskList, client_message, server_message,
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

    /// Handle `ServiceQuery` — route to a named service.
    fn service_query(
        &self,
        service: String,
        query: String,
    ) -> impl std::future::Future<Output = Result<String>> + Send;

    /// Handle `GetServiceSchema` — return JSON Schema for one service's config.
    fn get_service_schema(
        &self,
        service: String,
    ) -> impl std::future::Future<Output = Result<String>> + Send;

    /// Handle `GetAllSchemas` — return JSON Schemas for all services.
    fn get_all_schemas(
        &self,
    ) -> impl std::future::Future<Output = Result<std::collections::HashMap<String, String>>> + Send;

    /// Handle `GetServices` — list registered services with status.
    fn list_services(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<ServiceInfoMsg>>> + Send;

    /// Handle `SetServiceConfig` — update a single service's config.
    fn set_service_config(
        &self,
        service: String,
        config: String,
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
                client_message::Msg::ServiceQuery(sq) => {
                    yield match self.service_query(sq.service, sq.query).await {
                        Ok(result) => ServerMessage {
                            msg: Some(server_message::Msg::ServiceQueryResult(
                                ServiceQueryResultMsg { result },
                            )),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::GetServiceSchema(req) => {
                    let service = req.service;
                    yield match self.get_service_schema(service.clone()).await {
                        Ok(schema) => ServerMessage {
                            msg: Some(server_message::Msg::ServiceSchema(ServiceSchemaMsg {
                                service,
                                schema,
                            })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::GetAllSchemas(_) => {
                    yield match self.get_all_schemas().await {
                        Ok(schemas) => ServerMessage {
                            msg: Some(server_message::Msg::AllSchemas(AllSchemasMsg { schemas })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::GetServices(_) => {
                    yield match self.list_services().await {
                        Ok(services) => ServerMessage {
                            msg: Some(server_message::Msg::ServiceList(ServiceListMsg { services })),
                        },
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
                client_message::Msg::SetServiceConfig(req) => {
                    yield match self.set_service_config(req.service, req.config).await {
                        Ok(()) => server_pong(),
                        Err(e) => server_error(500, e.to_string()),
                    };
                }
            }
        }
    }
}
