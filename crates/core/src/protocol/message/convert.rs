//! Conversions between protocol message types.

use crate::agent::AgentConfig;
use crate::protocol::proto::{
    AgentEventMsg, AgentInfo, ClientMessage, ConversationHistory, McpEventMsg, PluginEvent,
    ReplyToAsk, SendMsg, SendResponse, ServerMessage, StreamEvent, StreamMsg, client_message,
    plugin_event, server_message, stream_event,
};

impl From<&AgentConfig> for AgentInfo {
    fn from(config: &AgentConfig) -> Self {
        Self {
            name: config.name.clone(),
            description: config.description.clone(),
            config: serde_json::to_string(config).unwrap_or_default(),
            model: config.model.clone(),
            max_iterations: config.max_iterations as u32,
            thinking: config.thinking,
            skills: config.skills.clone(),
            mcps: config.mcps.iter().map(|m| m.name.clone()).collect(),
            compact_tool_max_len: config.compact_tool_max_len as u32,
        }
    }
}

// ── ClientMessage constructors ───────────────────────────────────

impl From<SendMsg> for ClientMessage {
    fn from(msg: SendMsg) -> Self {
        Self {
            msg: Some(client_message::Msg::Send(msg)),
        }
    }
}

impl From<StreamMsg> for ClientMessage {
    fn from(msg: StreamMsg) -> Self {
        Self {
            msg: Some(client_message::Msg::Stream(msg)),
        }
    }
}

impl From<ReplyToAsk> for ClientMessage {
    fn from(msg: ReplyToAsk) -> Self {
        Self {
            msg: Some(client_message::Msg::ReplyToAsk(msg)),
        }
    }
}

// ── ServerMessage constructors ───────────────────────────────────

impl From<SendResponse> for ServerMessage {
    fn from(r: SendResponse) -> Self {
        Self {
            msg: Some(server_message::Msg::Response(r)),
        }
    }
}

impl From<StreamEvent> for ServerMessage {
    fn from(e: StreamEvent) -> Self {
        Self {
            msg: Some(server_message::Msg::Stream(e)),
        }
    }
}

impl From<AgentEventMsg> for ServerMessage {
    fn from(e: AgentEventMsg) -> Self {
        Self {
            msg: Some(server_message::Msg::AgentEvent(e)),
        }
    }
}

impl From<McpEventMsg> for ServerMessage {
    fn from(e: McpEventMsg) -> Self {
        Self {
            msg: Some(server_message::Msg::McpEvent(e)),
        }
    }
}

impl From<PluginEvent> for ServerMessage {
    fn from(e: PluginEvent) -> Self {
        Self {
            msg: Some(server_message::Msg::PluginEvent(e)),
        }
    }
}

impl From<ConversationHistory> for ServerMessage {
    fn from(h: ConversationHistory) -> Self {
        Self {
            msg: Some(server_message::Msg::ConversationHistory(h)),
        }
    }
}

// ── TryFrom<ServerMessage> ───────────────────────────────────────

fn error_or_unexpected(msg: ServerMessage) -> anyhow::Error {
    match msg.msg {
        Some(server_message::Msg::Error(e)) => {
            anyhow::anyhow!("server error ({}): {}", e.code, e.message)
        }
        other => anyhow::anyhow!("unexpected response: {other:?}"),
    }
}

impl TryFrom<ServerMessage> for SendResponse {
    type Error = anyhow::Error;
    fn try_from(msg: ServerMessage) -> anyhow::Result<Self> {
        match msg.msg {
            Some(server_message::Msg::Response(r)) => Ok(r),
            _ => Err(error_or_unexpected(msg)),
        }
    }
}

impl TryFrom<ServerMessage> for stream_event::Event {
    type Error = anyhow::Error;
    fn try_from(msg: ServerMessage) -> anyhow::Result<Self> {
        match msg.msg {
            Some(server_message::Msg::Stream(e)) => {
                e.event.ok_or_else(|| anyhow::anyhow!("empty stream event"))
            }
            _ => Err(error_or_unexpected(msg)),
        }
    }
}

impl TryFrom<ServerMessage> for plugin_event::Event {
    type Error = anyhow::Error;
    fn try_from(msg: ServerMessage) -> anyhow::Result<Self> {
        match msg.msg {
            Some(server_message::Msg::PluginEvent(e)) => {
                e.event.ok_or_else(|| anyhow::anyhow!("empty plugin event"))
            }
            _ => Err(error_or_unexpected(msg)),
        }
    }
}
