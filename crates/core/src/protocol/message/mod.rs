//! Wire protocol message types — re-exported from generated protobuf types.

mod convert;

pub use crate::protocol::proto::{
    AgentEventKind, client_message, hub_event, server_message, stream_event,
};
pub use crate::protocol::proto::{
    AgentEventMsg, AgentInfo, AgentList, AskOption, AskQuestion, AskUserEvent, ClientMessage,
    CompactMsg, CompactResponse, ConfigMsg, ConversationInfo, ConversationList, CreateAgentMsg,
    CreateCronMsg, CronInfo, CronList, DaemonStats, DeleteAgentMsg, DeleteCronMsg, ErrorMsg,
    GetAgentMsg, GetConfig, GetStats, HubDone, HubEvent, HubSetupOutput, HubStep, HubWarning,
    InstallPackageMsg, KillMsg, ListAgentsMsg, ListConversationsMsg, ListCronsMsg, ListPackagesMsg,
    ListProvidersMsg, ListSkillsMsg, PackageInfo, PackageList, Ping, Pong, ProviderInfo,
    ProviderList, ReplyToAsk, SendMsg, SendResponse, ServerMessage, ServiceLogOutput,
    ServiceLogsMsg, SessionInfo, SessionList, SetConfigMsg, SkillList, StartServiceMsg,
    StopServiceMsg, StreamChunk, StreamEnd, StreamEvent, StreamMsg, StreamStart, StreamThinking,
    SubscribeEvents, TokenUsage, ToolCallInfo, ToolResultEvent, ToolStartEvent, ToolsCompleteEvent,
    UninstallPackageMsg, UpdateAgentMsg,
};
