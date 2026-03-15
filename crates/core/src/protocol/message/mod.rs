//! Wire protocol message types — re-exported from generated protobuf types.

mod convert;

pub use crate::protocol::proto::{
    AllSchemasMsg, ApproveMsg, ClientMessage, ConfigMsg, DownloadCompleted, DownloadCreated,
    DownloadEvent, DownloadFailed, DownloadInfo, DownloadList, DownloadProgress, DownloadStep,
    Downloads, ErrorMsg, GetAllSchemasMsg, GetConfig, GetServiceSchemaMsg, GetServicesMsg,
    HubAction, HubMsg, KillMsg, KillTaskMsg, Ping, Pong, SendMsg, SendResponse, ServerMessage,
    ServiceInfoMsg, ServiceListMsg, ServiceQueryMsg, ServiceQueryResultMsg, ServiceSchemaMsg,
    SessionInfo, SessionList, SetConfigMsg, SetServiceConfigMsg, StreamChunk, StreamEnd,
    StreamEvent, StreamMsg, StreamStart, StreamThinking, SubscribeDownloads, SubscribeTasks,
    TaskCompleted, TaskCreated, TaskEvent, TaskInfo, TaskList, TaskStatusChanged, Tasks,
    ToolCallInfo, ToolResultEvent, ToolStartEvent, ToolsCompleteEvent,
};
pub use crate::protocol::proto::{
    DownloadKind, client_message, download_event, server_message, stream_event, task_event,
};
