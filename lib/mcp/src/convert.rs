//! Wire conversions for this crate's types.
//!
//! These live here rather than beside the protocol handlers because
//! `McpEvent` and `ServerStatus` are ours: a caller shouldn't have to know
//! how to render them, and a new variant should break here, next to the
//! definition, rather than somewhere downstream.

use crate::handler::{McpEvent, ServerStatus};
use proto::{McpEventKind, McpEventMsg, McpStatus};

impl From<McpEvent> for McpEventMsg {
    fn from(event: McpEvent) -> Self {
        let timestamp = chrono::Utc::now().to_rfc3339();
        let (kind, agent, name, tools, error) = match event {
            McpEvent::Connecting { agent, name } => (
                McpEventKind::Connecting,
                agent,
                name,
                Vec::new(),
                String::new(),
            ),
            McpEvent::Connected { agent, name, tools } => {
                (McpEventKind::Connected, agent, name, tools, String::new())
            }
            McpEvent::Failed { agent, name, error } => {
                (McpEventKind::Failed, agent, name, Vec::new(), error)
            }
            McpEvent::Disconnected { agent, name } => (
                McpEventKind::Disconnected,
                agent,
                name,
                Vec::new(),
                String::new(),
            ),
        };
        Self {
            kind: kind.into(),
            name,
            tools,
            error,
            timestamp,
            agent,
        }
    }
}

impl From<ServerStatus> for McpStatus {
    fn from(status: ServerStatus) -> Self {
        match status {
            ServerStatus::Connecting => Self::Connecting,
            ServerStatus::Connected => Self::Connected,
            ServerStatus::Failed => Self::Failed,
            ServerStatus::Disconnected => Self::Disconnected,
        }
    }
}
