//! Messages sent by the client to the gateway.

use compact_str::CompactString;
use serde::{Deserialize, Serialize};

/// Resource kind for config proxy operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resource {
    /// MCP server configurations.
    Mcp,
    /// Loaded skills (read-only).
    Skill,
    /// Agent configurations.
    Agent,
    /// Remote model provider configurations.
    Provider,
}

/// Hub package action.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HubAction {
    /// Install a hub package.
    Install,
    /// Uninstall a hub package.
    Uninstall,
}

/// Send a message to an agent and receive a complete response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendRequest {
    /// Target agent identifier.
    pub agent: CompactString,
    /// Message content.
    pub content: String,
    /// Session to reuse. `None` creates a new session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<u64>,
    /// Sender identity (e.g. `"tg:12345"`, `"dc:67890"`). `None` = local.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<CompactString>,
}

/// Send a message to an agent and receive a streamed response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamRequest {
    /// Target agent identifier.
    pub agent: CompactString,
    /// Message content.
    pub content: String,
    /// Session to reuse. `None` creates a new session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<u64>,
    /// Sender identity (e.g. `"tg:12345"`, `"dc:67890"`). `None` = local.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<CompactString>,
}

/// Request download of a model's files with progress reporting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRequest {
    /// HuggingFace model ID.
    pub model: CompactString,
}

/// Install or uninstall a hub package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubRequest {
    /// Package identifier in `scope/name` format.
    pub package: CompactString,
    /// Action to perform.
    pub action: HubAction,
}

/// Messages sent by the client to the gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Send a message to an agent and receive a complete response.
    Send {
        /// Target agent identifier.
        agent: CompactString,
        /// Message content.
        content: String,
        /// Session to reuse. `None` creates a new session.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<u64>,
        /// Sender identity. `None` = local.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sender: Option<CompactString>,
    },
    /// Send a message to an agent and receive a streamed response.
    Stream {
        /// Target agent identifier.
        agent: CompactString,
        /// Message content.
        content: String,
        /// Session to reuse. `None` creates a new session.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<u64>,
        /// Sender identity. `None` = local.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sender: Option<CompactString>,
    },
    /// Request download of a model's files with progress reporting.
    Download {
        /// HuggingFace model ID (e.g. "microsoft/Phi-3.5-mini-instruct").
        model: CompactString,
    },
    /// Ping the server (keepalive).
    Ping,
    /// Install or uninstall a hub package.
    Hub {
        /// Package identifier in `scope/name` format.
        package: CompactString,
        /// Action to perform.
        action: HubAction,
    },
    /// List active sessions.
    Sessions,
    /// Kill (close) a session.
    Kill {
        /// Session ID to close.
        session: u64,
    },
    /// List tasks in the task registry.
    Tasks,
    /// Kill (cancel) a task.
    KillTask {
        /// Task ID to cancel.
        task_id: u64,
    },
    /// Approve a blocked task's inbox item.
    Approve {
        /// Task ID to approve.
        task_id: u64,
        /// Response to send to the blocked tool call.
        response: String,
    },
    /// Evaluate whether the agent should respond to a message.
    ///
    /// Used by channel loops for group-chat gating (DD#39). Returns
    /// `ServerMessage::Evaluation` with a boolean decision.
    Evaluate {
        /// Target agent identifier.
        agent: CompactString,
        /// Message content to evaluate.
        content: String,
        /// Session to use for context. `None` creates a temporary session.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<u64>,
        /// Sender identity. `None` = local.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sender: Option<CompactString>,
    },
    /// Subscribe to task lifecycle events (streaming).
    SubscribeTasks,
    /// List downloads in the download registry.
    Downloads,
    /// Subscribe to download lifecycle events (streaming).
    SubscribeDownloads,
    /// List resources of a given kind.
    List {
        /// Resource kind to list.
        resource: Resource,
    },
    /// Add or update a named resource.
    AddResource {
        /// Resource kind.
        resource: Resource,
        /// Resource name (map key in config).
        name: String,
        /// JSON-serialized config value.
        value: String,
    },
    /// Remove a named resource.
    RemoveResource {
        /// Resource kind.
        resource: Resource,
        /// Resource name to remove.
        name: String,
    },
}

impl From<SendRequest> for ClientMessage {
    fn from(r: SendRequest) -> Self {
        Self::Send {
            agent: r.agent,
            content: r.content,
            session: r.session,
            sender: r.sender,
        }
    }
}

impl From<StreamRequest> for ClientMessage {
    fn from(r: StreamRequest) -> Self {
        Self::Stream {
            agent: r.agent,
            content: r.content,
            session: r.session,
            sender: r.sender,
        }
    }
}

impl From<DownloadRequest> for ClientMessage {
    fn from(r: DownloadRequest) -> Self {
        Self::Download { model: r.model }
    }
}

impl From<HubRequest> for ClientMessage {
    fn from(r: HubRequest) -> Self {
        Self::Hub {
            package: r.package,
            action: r.action,
        }
    }
}
