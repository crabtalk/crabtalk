//! Tool registry (schema store), ToolRequest, and ToolSender.
//!
//! [`ToolRegistry`] stores tool schemas by name — no handlers, no closures.
//! [`ToolRequest`] and [`ToolSender`] are the agent-side dispatch primitives:
//! the agent sends a `ToolRequest` per tool call and awaits a `String` reply.

use crate::model::Tool;
use compact_str::CompactString;
use std::collections::BTreeMap;
use tokio::sync::{mpsc, oneshot};

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
}

/// Sender half of the agent tool channel.
///
/// Captured by `Agent` at construction. When the model returns tool calls,
/// the agent sends one `ToolRequest` per call and awaits each reply.
/// `None` means no tools are available (e.g. CLI path without a daemon).
pub type ToolSender = mpsc::UnboundedSender<ToolRequest>;

/// Schema-only registry of named tools.
///
/// Stores `Tool` definitions (name, description, JSON schema) keyed by name.
/// Used by `Runtime` to filter tool schemas per agent at `add_agent` time.
/// No handlers or closures are stored here.
#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: BTreeMap<CompactString, Tool>,
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
    pub fn filtered_snapshot(&self, names: &[CompactString]) -> Vec<Tool> {
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
