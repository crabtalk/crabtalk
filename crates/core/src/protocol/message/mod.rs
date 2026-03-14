//! Wire protocol message types — re-exported from generated protobuf types.

mod convert;

pub use crate::protocol::proto::{
    ApproveMsg, ClientMessage, ConfigMsg, DownloadCompleted, DownloadCreated, DownloadEvent,
    DownloadFailed, DownloadInfo, DownloadList, DownloadProgress, DownloadStep, Downloads,
    EntityInfo, EntityList, ErrorMsg, EvaluateMsg, EvaluationMsg, GetConfig, HubAction, HubMsg,
    JournalInfo, JournalList, KillMsg, KillTaskMsg, MemoryEntities, MemoryJournals, MemoryOp,
    MemoryQueryMsg, MemoryRelations, MemoryResult, MemorySearch, Ping, Pong, RelationInfo,
    RelationList, SendMsg, SendResponse, ServerMessage, SessionInfo, SessionList, SetConfigMsg,
    StreamChunk, StreamEnd, StreamEvent, StreamMsg, StreamStart, StreamThinking,
    SubscribeDownloads, SubscribeTasks, TaskCompleted, TaskCreated, TaskEvent, TaskInfo, TaskList,
    TaskStatusChanged, Tasks, ToolCallInfo, ToolResultEvent, ToolStartEvent, ToolsCompleteEvent,
};
pub use crate::protocol::proto::{
    DownloadKind, client_message, download_event, memory_op, memory_result, server_message,
    stream_event, task_event,
};
