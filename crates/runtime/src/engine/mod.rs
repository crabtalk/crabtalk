//! Runtime — agent registry and agent execution.
//!
//! [`Runtime`] holds agents as immutable definitions. Tool schemas and
//! handlers are registered by the caller at construction. Execution
//! methods (`send_to`, `stream_to`) are handed the session to run
//! against; which sessions are live, and under what id, is the
//! caller's bookkeeping — the runtime is rebuilt on every config reload
//! and a session outlives that.

use crate::{Config, Env, Harness};
use crate::{ToolRegistry, agent::Model};
use std::sync::Arc;

mod agents;
mod config;
mod execution;
mod history;
mod session;

/// The crabtalk runtime.
///
/// Holds interfaces, not data. There is no agent registry and no
/// resident memory here: an agent is built from storage for the run that
/// needs it and dropped after, and memory is a store the interface
/// reaches. Whether any of it is cached is the backend's decision, which
/// is what lets a different deployment be a different implementation
/// rather than a rewrite of this file.
pub struct Runtime<C: Config> {
    pub model: Model<C::Provider>,
    pub env: Arc<C::Env>,
    storage: Arc<C::Storage>,
    pub tools: ToolRegistry,
    /// Model names advertised by the LLM endpoint — populated by the
    /// daemon builder from a `/v1/models` fetch at startup / reload.
    pub(super) models: parking_lot::RwLock<Vec<String>>,
}

impl<C: Config> Runtime<C> {
    /// Create a new runtime with the given model, env, storage, and tools.
    pub fn new(
        model: Model<C::Provider>,
        env: Arc<C::Env>,
        storage: Arc<C::Storage>,
        tools: ToolRegistry,
    ) -> Self {
        Self {
            model,
            env,
            storage,
            tools,
            models: parking_lot::RwLock::new(Vec::new()),
        }
    }

    /// Access the persistence backend.
    pub fn storage(&self) -> &Arc<C::Storage> {
        &self.storage
    }

    /// How a surface opts into capabilities held out of the ambient tool set,
    /// passed as a stream's `extra_tools`.
    pub fn scoped_tools(&self, names: &[String]) -> Vec<crabllm_core::Tool> {
        self.env.hook().scoped_schema(names)
    }
}
