//! Tool schema for the delegate tool.
//!
//! Schema types live here. Dispatch logic is server-specific and lives in
//! the [`Host`](crate::host::Host) implementation.

use serde::Deserialize;
use wcore::{
    agent::{AsTool, ToolDescription},
    model::Tool,
};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Delegate {
    /// List of tasks to run in parallel. Each task has an agent name and a message.
    pub tasks: Vec<DelegateTask>,
    /// If true, return immediately with task IDs instead of waiting for completion.
    /// Results arrive via agent completion events (`agent:{name}:done`).
    #[serde(default)]
    pub background: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DelegateTask {
    /// Target agent name. Auto-generated if empty and system_prompt is set.
    #[serde(default)]
    pub agent: String,
    /// Message/instruction for the target agent.
    pub message: String,
    /// System prompt for an ephemeral agent. When set, creates a temporary
    /// agent with this prompt instead of targeting a pre-registered agent.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Working directory for this task. Defaults to the parent's CWD.
    #[serde(default)]
    pub cwd: Option<String>,
}

impl ToolDescription for Delegate {
    const DESCRIPTION: &'static str = "Delegate tasks to other agents. Runs all tasks in parallel. Set background=true to return immediately with task IDs — results arrive via agent completion events.";
}

pub fn tools() -> Vec<Tool> {
    vec![Delegate::as_tool()]
}
