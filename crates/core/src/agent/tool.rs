//! Tool registry (schema store), ToolRequest, and ToolSender.
//!
//! [`ToolRegistry`] stores tool schemas by name — no handlers, no closures.
//! [`ToolRequest`] and [`ToolSender`] are the agent-side dispatch primitives:
//! the agent sends a `ToolRequest` per tool call and awaits a `String` reply.

use crate::model::Tool;
use heck::ToSnakeCase;
use schemars::JsonSchema;
use std::collections::BTreeMap;
use tokio::sync::{mpsc, oneshot};

/// Sender half of the agent tool channel.
///
/// Captured by `Agent` at construction. When the model returns tool calls,
/// the agent sends one `ToolRequest` per call and awaits each reply.
/// `None` means no tools are available (e.g. CLI path without a daemon).
pub type ToolSender = mpsc::UnboundedSender<ToolRequest>;

/// A single tool call request sent by the agent to the runtime's tool handler.
pub struct ToolRequest {
    /// Tool name as returned by the model.
    pub name: String,
    /// JSON-encoded arguments string.
    pub args: String,
    /// Name of the agent that made this call.
    pub agent: String,
    /// Reply channel — the handler sends the result string here.
    pub reply: oneshot::Sender<String>,
    /// Task ID of the calling task, if running within a task context.
    /// Set by the daemon when dispatching task-bound tool calls.
    pub task_id: Option<u64>,
    /// Sender identity of the user who triggered this agent run.
    /// Empty for local/owner conversations.
    pub sender: String,
    /// Conversation ID, if running within a conversation.
    /// Set by the runtime; the agent passes it through as an opaque value.
    pub conversation_id: Option<u64>,
}

/// Schema-only registry of named tools.
///
/// Stores `Tool` definitions (name, description, JSON schema) keyed by name.
/// Used by `Runtime` to filter tool schemas per agent at `add_agent` time.
/// No handlers or closures are stored here.
#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Tool>,
}

impl ToolRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a tool schema.
    pub fn insert(&mut self, tool: Tool) {
        self.tools.insert(tool.name.clone(), tool);
    }

    /// Insert multiple tool schemas.
    pub fn insert_all(&mut self, tools: Vec<Tool>) {
        for tool in tools {
            self.insert(tool);
        }
    }

    /// Remove a tool by name. Returns `true` if it existed.
    pub fn remove(&mut self, name: &str) -> bool {
        self.tools.remove(name).is_some()
    }

    /// Check if a tool is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Return all tool schemas as a `Vec`.
    pub fn tools(&self) -> Vec<Tool> {
        self.tools.values().cloned().collect()
    }

    /// Build a filtered list of tool schemas matching the given names.
    ///
    /// If `names` is empty, all tools are returned. Used by `Runtime::add_agent`
    /// to build the per-agent schema snapshot stored on `Agent`.
    pub fn filtered_snapshot(&self, names: &[String]) -> Vec<Tool> {
        if names.is_empty() {
            return self.tools();
        }
        self.tools
            .iter()
            .filter(|(k, _)| names.iter().any(|n| n == *k))
            .map(|(_, v)| v.clone())
            .collect()
    }
}

/// Trait to provide a description for a tool.
pub trait ToolDescription {
    /// The description of the tool.
    const DESCRIPTION: &'static str;
}

/// Trait to convert a type into a tool.
pub trait AsTool: ToolDescription {
    /// Convert the type into a tool.
    fn as_tool() -> Tool;
}

impl<T> AsTool for T
where
    T: JsonSchema + ToolDescription,
{
    fn as_tool() -> Tool {
        Tool {
            name: T::schema_name().to_snake_case(),
            description: Self::DESCRIPTION.into(),
            parameters: schemars::schema_for!(T),
            strict: true,
        }
    }
}
