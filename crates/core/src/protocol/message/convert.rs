//! Conversions between protocol message types.

use crate::config::ApiStandard;
use crate::protocol::proto::{
    AgentEventMsg, ClientMessage, ConversationHistory, PluginEvent, ProviderKind, ReplyToAsk,
    SendMsg, SendResponse, ServerMessage, StreamEvent, StreamMsg, client_message, plugin_event,
    server_message, stream_event,
};

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

impl From<ApiStandard> for ProviderKind {
    fn from(kind: ApiStandard) -> Self {
        match kind {
            ApiStandard::Openai => Self::Openai,
            ApiStandard::Anthropic => Self::Anthropic,
            ApiStandard::Google => Self::Google,
            ApiStandard::Bedrock => Self::Bedrock,
            ApiStandard::Ollama => Self::Ollama,
            ApiStandard::Azure => Self::Azure,
            // Self-defined kinds dispatch through the OpenAI-compatible
            // path; the proto enum has no Custom variant, so report Openai.
            ApiStandard::Custom(_) => Self::Openai,
        }
    }
}

impl From<ProviderKind> for ApiStandard {
    fn from(kind: ProviderKind) -> Self {
        match kind {
            ProviderKind::Openai | ProviderKind::Unknown => Self::Openai,
            ProviderKind::Anthropic => Self::Anthropic,
            ProviderKind::Google => Self::Google,
            ProviderKind::Bedrock => Self::Bedrock,
            ProviderKind::Ollama => Self::Ollama,
            ProviderKind::Azure => Self::Azure,
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
