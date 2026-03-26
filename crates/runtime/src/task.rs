//! Tool schema for the delegate tool.
//!
//! Schema types live here. Dispatch logic is server-specific and lives in
//! the [`RuntimeBridge`](crate::bridge::RuntimeBridge) implementation.

use serde::Deserialize;
use wcore::{
    agent::{AsTool, ToolDescription},
    model::Tool,
};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Delegate {
    /// List of tasks to run in parallel. Each task has an agent name and a message.
    pub tasks: Vec<DelegateTask>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DelegateTask {
    /// Target agent name.
    pub agent: String,
    /// Message/instruction for the target agent.
    pub message: String,
}

impl ToolDescription for Delegate {
    const DESCRIPTION: &'static str = "Delegate tasks to other agents. Runs all tasks in parallel, blocks until all complete, and returns their results.";
}

pub fn tools() -> Vec<Tool> {
    vec![Delegate::as_tool()]
}
