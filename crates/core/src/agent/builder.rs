//! Fluent builder for constructing an [`Agent`].

use super::tool::ToolSender;
use crate::{
    agent::{Agent, config::AgentConfig},
    model::{Model, Tool},
};

/// Fluent builder for [`Agent<M>`].
///
/// Requires a model at construction. Use [`AgentConfig`] builder methods
/// for field configuration, then pass it via [`AgentBuilder::config`].
pub struct AgentBuilder<M: Model> {
    config: AgentConfig,
    model: M,
    tools: Vec<Tool>,
    tool_tx: Option<ToolSender>,
}

impl<M: Model> AgentBuilder<M> {
    /// Create a new builder with the given model.
    pub fn new(model: M) -> Self {
        Self {
            config: AgentConfig::default(),
            model,
            tools: Vec::new(),
            tool_tx: None,
        }
    }

    /// Set the full config, replacing all fields.
    pub fn config(mut self, config: AgentConfig) -> Self {
        self.config = config;
        self
    }

    /// Set the tool schemas advertised to the LLM.
    pub fn tools(mut self, tools: Vec<Tool>) -> Self {
        self.tools = tools;
        self
    }

    /// Set the tool sender for dispatching tool calls.
    pub fn tool_tx(mut self, tx: ToolSender) -> Self {
        self.tool_tx = Some(tx);
        self
    }

    /// Build the [`Agent`].
    pub fn build(self) -> Agent<M> {
        Agent {
            config: self.config,
            model: self.model,
            history: Vec::new(),
            tools: self.tools,
            tool_tx: self.tool_tx,
        }
    }
}
