//! Conversions between protocol message types.

use crate::protocol::proto::{
    AgentEventMsg, ClientMessage, ReplyToAsk, SendMsg, SendResponse, ServerMessage, StreamEvent,
    StreamMsg, client_message, server_message, stream_event,
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
