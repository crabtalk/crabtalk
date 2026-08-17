//! Runtime — agent registry and agent execution.
//!
//! [`Runtime`] holds agents as immutable definitions. Tool schemas and
//! handlers are registered by the caller at construction. Execution
//! methods (`send_to`, `stream_to`) are handed the session to run
//! against; which sessions are live, and under what id, is the
//! caller's bookkeeping — the runtime is rebuilt on every config reload
//! and a session outlives that.

use crate::{Agent, ToolRegistry, agent::Model};
use crate::{Config, Env, Harness};
use memory::Memory;
use std::{collections::BTreeMap, sync::Arc};
use storage::AgentId;

mod agents;
mod config;
mod execution;
mod history;
mod session;

/// Shared handle to the standalone memory store. Used by compaction to
/// write Archive entries and by session resume to pull their content
/// back as the replayed prefix.
pub type SharedMemory = Arc<parking_lot::RwLock<Memory>>;

/// The crabtalk runtime.
pub struct Runtime<C: Config> {
    pub model: Model<C::Provider>,
    pub env: Arc<C::Env>,
    storage: Arc<C::Storage>,
    memory: SharedMemory,
    agents: parking_lot::RwLock<BTreeMap<AgentId, Agent<C::Provider>>>,
    pub tools: ToolRegistry,
    /// Model names advertised by the LLM endpoint — populated by the
    /// daemon builder from a `/v1/models` fetch at startup / reload.
    pub(super) models: parking_lot::RwLock<Vec<String>>,
}

impl<C: Config> Runtime<C> {
    /// Create a new runtime with the given model, env, storage, memory, and tools.
    pub fn new(
        model: Model<C::Provider>,
        env: Arc<C::Env>,
        storage: Arc<C::Storage>,
        memory: SharedMemory,
        tools: ToolRegistry,
    ) -> Self {
        Self {
            model,
            env,
            storage,
            memory,
            agents: parking_lot::RwLock::new(BTreeMap::new()),
            tools,
            models: parking_lot::RwLock::new(Vec::new()),
        }
    }

    /// Access the persistence backend.
    pub fn storage(&self) -> &Arc<C::Storage> {
        &self.storage
    }

    /// Access the shared memory store.
    pub fn memory(&self) -> &SharedMemory {
        &self.memory
    }

    /// How a surface opts into capabilities held out of the ambient tool set,
    /// passed as a stream's `extra_tools`.
    pub fn scoped_tools(&self, names: &[String]) -> Vec<crabllm_core::Tool> {
        self.env.hook().scoped_schema(names)
    }
}
