//! Wire protocol message types — re-exported from generated protobuf types.

mod convert;

pub use crate::protocol::proto::{AgentEventKind, client_message, server_message, stream_event};
pub use crate::protocol::proto::{
    AgentEventMsg, AgentInfo, AgentList, AskOption, AskQuestion, AskUserEvent, ClientMessage,
    CompactMsg, CompactResponse, ConfigMsg, CreateAgentMsg, CreateCronMsg, CronInfo, CronList,
    DaemonStats, DeleteAgentMsg, DeleteCronMsg, ErrorMsg, GetAgentMsg, GetConfig, GetStats,
    InstallPackageMsg, KillMsg, ListAgentsMsg, ListCronsMsg, ListPackagesMsg, ListProvidersMsg,
    PackageInfo, PackageList, Ping, Pong, ProviderInfo, ProviderList, ReplyToAsk, SendMsg,
    SendResponse, ServerMessage, SessionInfo, SessionList, SetConfigMsg, StreamChunk, StreamEnd,
    StreamEvent, StreamMsg, StreamStart, StreamThinking, SubscribeEvents, TokenUsage, ToolCallInfo,
    ToolResultEvent, ToolStartEvent, ToolsCompleteEvent, UninstallPackageMsg, UpdateAgentMsg,
};
