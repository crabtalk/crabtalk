//! Wire protocol message types — re-exported from generated protobuf types.

mod convert;

pub use crate::protocol::proto::{
    AgentEventKind, McpStatus, ProviderKind as ProtoProviderKind, SourceKind, client_message,
    hub_event, server_message, stream_event,
};
pub use crate::protocol::proto::{
    AgentEventMsg, AgentInfo, AgentList, AskOption, AskQuestion, AskUserEvent, ClientMessage,
    CompactMsg, CompactResponse, ConfigMsg, ConversationHistory, ConversationInfo,
    ConversationList, ConversationMessage, CreateAgentMsg, CreateCronMsg, CronInfo, CronList,
    DaemonStats, DeleteAgentMsg, DeleteConversationMsg, DeleteCronMsg, DeleteProviderMsg, ErrorMsg,
    GetAgentMsg, GetConfig, GetConversationHistoryMsg, GetStats, HubDone, HubEvent, HubPackageInfo,
    HubPackageList, HubSetupOutput, HubStep, HubWarning, InstallPackageMsg, KillMsg, ListAgentsMsg,
    ListConversationsMsg, ListCronsMsg, ListMcpsMsg, ListModelsMsg, ListPackagesMsg,
    ListProviderPresetsMsg, ListProvidersMsg, ListSkillsMsg, McpInfo, McpList, ModelInfo,
    ModelList, PackageInfo, PackageList, Ping, Pong, ProviderInfo, ProviderList,
    ProviderPresetInfo, ProviderPresetList, ReplyToAsk, ResourceKind, SearchHubMsg, SendMsg,
    SendResponse, ServerMessage, ServiceLogOutput, ServiceLogsMsg, SessionInfo, SessionList,
    SetActiveModelMsg, SetEnabledMsg, SetLocalMcpsMsg, SetProviderMsg, SkillInfo, SkillList,
    StartServiceMsg, StopServiceMsg, StreamChunk, StreamEnd, StreamEvent, StreamMsg, StreamStart,
    StreamThinking, SubscribeEvents, TokenUsage, ToolCallInfo, ToolResultEvent, ToolStartEvent,
    ToolsCompleteEvent, UninstallPackageMsg, UpdateAgentMsg,
};
