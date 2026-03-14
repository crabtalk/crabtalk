//! Conversions between protocol message types.

use crate::protocol::proto::{
    ClientMessage, DownloadEvent, SendMsg, SendResponse, ServerMessage, StreamEvent, StreamMsg,
    TaskEvent, client_message, download_event, memory_result, server_message, stream_event,
    task_event,
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

impl From<DownloadEvent> for ServerMessage {
    fn from(e: DownloadEvent) -> Self {
        Self {
            msg: Some(server_message::Msg::Download(e)),
        }
    }
}

impl From<TaskEvent> for ServerMessage {
    fn from(e: TaskEvent) -> Self {
        Self {
            msg: Some(server_message::Msg::Task(e)),
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

impl TryFrom<ServerMessage> for download_event::Event {
    type Error = anyhow::Error;
    fn try_from(msg: ServerMessage) -> anyhow::Result<Self> {
        match msg.msg {
            Some(server_message::Msg::Download(e)) => e
                .event
                .ok_or_else(|| anyhow::anyhow!("empty download event")),
            _ => Err(error_or_unexpected(msg)),
        }
    }
}

impl TryFrom<ServerMessage> for task_event::Event {
    type Error = anyhow::Error;
    fn try_from(msg: ServerMessage) -> anyhow::Result<Self> {
        match msg.msg {
            Some(server_message::Msg::Task(e)) => {
                e.event.ok_or_else(|| anyhow::anyhow!("empty task event"))
            }
            _ => Err(error_or_unexpected(msg)),
        }
    }
}

impl TryFrom<ServerMessage> for memory_result::Result {
    type Error = anyhow::Error;
    fn try_from(msg: ServerMessage) -> anyhow::Result<Self> {
        match msg.msg {
            Some(server_message::Msg::Memory(r)) => r
                .result
                .ok_or_else(|| anyhow::anyhow!("empty memory result")),
            _ => Err(error_or_unexpected(msg)),
        }
    }
}
