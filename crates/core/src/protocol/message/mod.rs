//! Wire protocol message types — re-exported from generated protobuf types.

mod convert;

pub use crate::protocol::proto::{AgentEventKind, client_message, server_message, stream_event};
pub use crate::protocol::proto::{
    AgentEventMsg, AskOption, AskQuestion, AskUserEvent, ClientMessage, CompactMsg,
    CompactResponse, ConfigMsg, CreateCronMsg, CronInfo, CronList, DaemonStats, DeleteCronMsg,
    ErrorMsg, GetConfig, GetStats, KillMsg, ListCronsMsg, Ping, Pong, ReplyToAsk, SendMsg,
    SendResponse, ServerMessage, SessionInfo, SessionList, SetConfigMsg, StreamChunk, StreamEnd,
    StreamEvent, StreamMsg, StreamStart, StreamThinking, SubscribeEvents, ToolCallInfo,
    ToolResultEvent, ToolStartEvent, ToolsCompleteEvent,
};
