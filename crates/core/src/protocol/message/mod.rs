//! Wire protocol message types — re-exported from generated protobuf types.

mod convert;

pub use crate::protocol::proto::{
    AgentEventKind, DownloadKind, HubAction, client_message, download_event, server_message,
    stream_event,
};
pub use crate::protocol::proto::{
    AgentEventMsg, AskOption, AskQuestion, AskUserEvent, ClientMessage, ConfigMsg,
    DownloadCompleted, DownloadCreated, DownloadEvent, DownloadFailed, DownloadInfo, DownloadList,
    DownloadProgress, DownloadStep, Downloads, ErrorMsg, GetConfig, HubMsg, KillMsg, Ping, Pong,
    ReplyToAsk, SendMsg, SendResponse, ServerMessage, SessionInfo, SessionList, SetConfigMsg,
    StreamChunk, StreamEnd, StreamEvent, StreamMsg, StreamStart, StreamThinking,
    SubscribeDownloads, SubscribeEvents, ToolCallInfo, ToolResultEvent, ToolStartEvent,
    ToolsCompleteEvent,
};
