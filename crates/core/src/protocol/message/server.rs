//! Messages sent by the gateway to the client.

use crate::{AgentConfig, McpServerConfig, ProviderConfig};
use compact_str::CompactString;
use serde::{Deserialize, Serialize};

/// Complete response from an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendResponse {
    /// Source agent identifier.
    pub agent: CompactString,
    /// Response content.
    pub content: String,
    /// Session ID used for this request.
    pub session: u64,
}

/// Lightweight tool call info for streaming events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallInfo {
    /// Tool name.
    pub name: CompactString,
    /// Tool arguments (JSON string).
    pub arguments: String,
}

/// Events emitted during a streamed agent response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamEvent {
    /// Stream has started.
    Start {
        /// Source agent identifier.
        agent: CompactString,
        /// Session ID used for this stream.
        session: u64,
    },
    /// A chunk of streamed content.
    Chunk {
        /// Chunk content.
        content: String,
    },
    /// A chunk of thinking/reasoning content.
    Thinking {
        /// Thinking content.
        content: String,
    },
    /// Agent started executing tool calls.
    ToolStart {
        /// Tool calls being executed.
        calls: Vec<ToolCallInfo>,
    },
    /// A single tool call completed.
    ToolResult {
        /// The tool call ID.
        call_id: CompactString,
        /// Tool output.
        output: String,
    },
    /// All tool calls completed.
    ToolsComplete,
    /// Stream has ended.
    End {
        /// Source agent identifier.
        agent: CompactString,
    },
}

/// Kind of download operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadKind {
    /// Local model download from HuggingFace.
    Model,
    /// Hub package install/uninstall.
    Hub,
    /// Embeddings model pre-download.
    Embeddings,
    /// Skill download (future).
    Skill,
}

impl std::fmt::Display for DownloadKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Model => write!(f, "model"),
            Self::Hub => write!(f, "hub"),
            Self::Embeddings => write!(f, "embeddings"),
            Self::Skill => write!(f, "skill"),
        }
    }
}

/// Unified download lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DownloadEvent {
    /// A new download was registered.
    Created {
        /// Download identifier.
        id: u64,
        /// Kind of download.
        kind: DownloadKind,
        /// Human-readable label (model ID, package name, etc.).
        label: String,
    },
    /// Byte-level progress (delta, not cumulative).
    Progress {
        /// Download identifier.
        id: u64,
        /// Bytes downloaded in this chunk.
        bytes: u64,
        /// Total expected bytes (0 if unknown).
        total_bytes: u64,
    },
    /// Human-readable progress step.
    Step {
        /// Download identifier.
        id: u64,
        /// Step description.
        message: String,
    },
    /// Download completed successfully.
    Completed {
        /// Download identifier.
        id: u64,
    },
    /// Download failed.
    Failed {
        /// Download identifier.
        id: u64,
        /// Error message.
        error: String,
    },
}

/// Summary of a download in the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadInfo {
    /// Download identifier.
    pub id: u64,
    /// Kind of download.
    pub kind: DownloadKind,
    /// Human-readable label.
    pub label: String,
    /// Current status.
    pub status: String,
    /// Bytes downloaded so far.
    pub bytes_downloaded: u64,
    /// Total expected bytes (0 if unknown).
    pub total_bytes: u64,
    /// Error message (if failed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Seconds since download started.
    pub alive_secs: u64,
}

/// Summary of an active session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Session identifier.
    pub id: u64,
    /// Agent this session is bound to.
    pub agent: CompactString,
    /// Origin of this session.
    pub created_by: CompactString,
    /// Number of messages in history.
    pub message_count: usize,
    /// Seconds since session was created.
    pub alive_secs: u64,
}

/// Summary of a task in the task registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    /// Task identifier.
    pub id: u64,
    /// Parent task ID (if sub-task).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<u64>,
    /// Agent assigned to this task.
    pub agent: CompactString,
    /// Current status.
    pub status: String,
    /// Task description / message.
    pub description: String,
    /// Result content (if finished).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// Error message (if failed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Origin of this task.
    pub created_by: CompactString,
    /// Cumulative prompt tokens.
    pub prompt_tokens: u64,
    /// Cumulative completion tokens.
    pub completion_tokens: u64,
    /// Seconds since task was created.
    pub alive_secs: u64,
    /// Pending inbox question (if blocked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_on: Option<String>,
}

/// Task lifecycle events emitted by the subscription stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskEvent {
    /// A new task was created.
    Created {
        /// Full task snapshot at creation time.
        task: TaskInfo,
    },
    /// Task status changed (non-terminal).
    StatusChanged {
        /// Task identifier.
        task_id: u64,
        /// New status.
        status: String,
        /// Pending inbox question (if blocked).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        blocked_on: Option<String>,
    },
    /// Task reached a terminal state (finished or failed).
    Completed {
        /// Task identifier.
        task_id: u64,
        /// Terminal status ("finished" or "failed").
        status: String,
        /// Result content (if finished).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        /// Error message (if failed).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

/// Summary of a loaded skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInfo {
    /// Skill identifier.
    pub name: CompactString,
    /// Human-readable description.
    pub description: String,
    /// License name or reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<CompactString>,
    /// Compatibility constraints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<CompactString>,
}

/// Typed resource list response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceList {
    /// MCP server configurations.
    Mcps(Vec<(CompactString, McpServerConfig)>),
    /// Loaded skill summaries (read-only).
    Skills(Vec<SkillInfo>),
    /// Agent configurations.
    Agents(Vec<(CompactString, AgentConfig)>),
    /// Remote model provider configurations.
    Providers(Vec<(CompactString, ProviderConfig)>),
}

/// Messages sent by the gateway to the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Complete response from an agent.
    Response(SendResponse),
    /// A streamed response event.
    Stream(StreamEvent),
    /// A download lifecycle event.
    Download(DownloadEvent),
    /// Error response.
    Error {
        /// Error code.
        code: u16,
        /// Error message.
        message: String,
    },
    /// Pong response to client ping.
    Pong,
    /// Active session list.
    Sessions(Vec<SessionInfo>),
    /// Download registry list.
    Downloads(Vec<DownloadInfo>),
    /// Task registry list.
    Tasks(Vec<TaskInfo>),
    /// A task lifecycle event (subscription stream).
    Task(TaskEvent),
    /// Evaluation result — whether the agent should respond (DD#39).
    Evaluation {
        /// Whether the agent decided to respond.
        respond: bool,
    },
    /// Resource list response.
    Resource(ResourceList),
}

impl From<SendResponse> for ServerMessage {
    fn from(r: SendResponse) -> Self {
        Self::Response(r)
    }
}

impl From<StreamEvent> for ServerMessage {
    fn from(e: StreamEvent) -> Self {
        Self::Stream(e)
    }
}

impl From<DownloadEvent> for ServerMessage {
    fn from(e: DownloadEvent) -> Self {
        Self::Download(e)
    }
}

impl From<TaskEvent> for ServerMessage {
    fn from(e: TaskEvent) -> Self {
        Self::Task(e)
    }
}

fn error_or_unexpected(msg: ServerMessage) -> anyhow::Error {
    match msg {
        ServerMessage::Error { code, message } => {
            anyhow::anyhow!("server error ({code}): {message}")
        }
        other => anyhow::anyhow!("unexpected response: {other:?}"),
    }
}

impl TryFrom<ServerMessage> for SendResponse {
    type Error = anyhow::Error;
    fn try_from(msg: ServerMessage) -> anyhow::Result<Self> {
        match msg {
            ServerMessage::Response(r) => Ok(r),
            other => Err(error_or_unexpected(other)),
        }
    }
}

impl TryFrom<ServerMessage> for StreamEvent {
    type Error = anyhow::Error;
    fn try_from(msg: ServerMessage) -> anyhow::Result<Self> {
        match msg {
            ServerMessage::Stream(e) => Ok(e),
            other => Err(error_or_unexpected(other)),
        }
    }
}

impl TryFrom<ServerMessage> for DownloadEvent {
    type Error = anyhow::Error;
    fn try_from(msg: ServerMessage) -> anyhow::Result<Self> {
        match msg {
            ServerMessage::Download(e) => Ok(e),
            other => Err(error_or_unexpected(other)),
        }
    }
}

impl TryFrom<ServerMessage> for TaskEvent {
    type Error = anyhow::Error;
    fn try_from(msg: ServerMessage) -> anyhow::Result<Self> {
        match msg {
            ServerMessage::Task(e) => Ok(e),
            other => Err(error_or_unexpected(other)),
        }
    }
}
