//! Fluent builder for constructing an [`Agent`].

use crate::{
    agent::{Agent, config::AgentConfig, tool::ToolDispatcher},
    model::Model,
};
use crabllm_core::{Provider, Tool};
use std::sync::Arc;

/// Fluent builder for [`Agent<P>`].
///
/// Requires a model at construction. Use [`AgentConfig`] builder methods
/// for field configuration, then pass it via [`AgentBuilder::config`].
pub struct AgentBuilder<P: Provider + 'static> {
    config: AgentConfig,
    model: Model<P>,
    tools: Vec<Tool>,
    dispatcher: Option<Arc<dyn ToolDispatcher>>,
}

impl<P: Provider + 'static> AgentBuilder<P> {
    /// Create a new builder with the given model.
    pub fn new(model: Model<P>) -> Self {
        Self {
            config: AgentConfig::default(),
            model,
            tools: Vec::new(),
            dispatcher: None,
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

    /// Set the tool dispatcher for executing tool calls.
    pub fn dispatcher(mut self, dispatcher: Arc<dyn ToolDispatcher>) -> Self {
        self.dispatcher = Some(dispatcher);
        self
    }

    /// Build the [`Agent`].
    pub fn build(self) -> Agent<P> {
        Agent {
            config: self.config,
            model: self.model,
            tools: self.tools,
            dispatcher: self.dispatcher,
        }
    }
}
