//! Wire protocol message types — re-exported from generated protobuf types.

mod convert;

pub use crate::protocol::proto::{
    ActiveConversationInfo, ActiveConversationList, AgentEventMsg, AgentInfo, AgentList, AskOption,
    AskQuestion, AskUserEvent, ClientMessage, CompactMsg, CompactResponse, ConfigMsg,
    ConversationHistory, ConversationInfo, ConversationList, ConversationMessage, CreateAgentMsg,
    CreateCronMsg, CronInfo, CronList, DaemonStats, DeleteAgentMsg, DeleteConversationMsg,
    DeleteCronMsg, DeleteProviderMsg, ErrorMsg, GetAgentMsg, GetConfig, GetConversationHistoryMsg,
    GetStats, InstallPluginMsg, KillMsg, ListActiveConversationsMsg, ListAgentsMsg,
    ListConversationsMsg, ListCronsMsg, ListMcpsMsg, ListModelsMsg, ListPluginsMsg,
    ListProviderPresetsMsg, ListProvidersMsg, ListSkillsMsg, ListSubscriptionsMsg, McpInfo,
    McpList, ModelInfo, ModelList, Ping, PluginDone, PluginEvent, PluginInfo, PluginList,
    PluginSearchList, PluginSetupOutput, PluginStep, PluginWarning, Pong, ProviderInfo,
    ProviderList, ProviderPresetInfo, ProviderPresetList, PublishEventMsg, ReplyToAsk,
    ResourceKind, SearchPluginsMsg, SendMsg, SendResponse, ServerMessage, ServiceLogOutput,
    ServiceLogsMsg, SetActiveModelMsg, SetEnabledMsg, SetLocalMcpsMsg, SetProviderMsg, SkillInfo,
    SkillList, StartServiceMsg, SteerSessionMsg, StopServiceMsg, StreamChunk, StreamEnd,
    StreamEvent, StreamMsg, StreamStart, StreamThinking, SubscribeEventMsg, SubscribeEvents,
    SubscriptionInfo, SubscriptionList, TextEndEvent, TextStartEvent, ThinkingEndEvent,
    ThinkingStartEvent, TokenUsage, ToolCallInfo, ToolResultEvent, ToolStartEvent,
    ToolsCompleteEvent, UninstallPluginMsg, UnsubscribeEventMsg, UpdateAgentMsg, UserSteeredEvent,
};
pub use crate::protocol::proto::{
    AgentEventKind, McpStatus, ProviderKind as ProtoProviderKind, SourceKind, client_message,
    plugin_event, server_message, stream_event,
};
