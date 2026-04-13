//! Runtime — agent registry, conversation management, and hook orchestration.
//!
//! [`Runtime`] holds agents as immutable definitions and conversations as
//! per-conversation `Arc<Mutex<Conversation>>` containers. Tool schemas and
//! handlers are registered by the caller at construction. Execution methods
//! (`send_to`, `stream_to`) take a conversation ID, lock the conversation,
//! clone the agent, and run with the conversation's history.

mod agents;
mod conversation;
mod execution;

use crate::{Config, Conversation};
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock as StdRwLock, atomic::AtomicU64},
};
use tokio::sync::{Mutex, RwLock, watch};
use wcore::{Agent, ToolRegistry, model::Model};

/// The crabtalk runtime.
pub struct Runtime<C: Config> {
    pub model: Model<C::Provider>,
    pub env: Arc<C::Env>,
    storage: Arc<C::Storage>,
    agents: StdRwLock<BTreeMap<String, Agent<C::Provider>>>,
    ephemeral_agents: RwLock<BTreeMap<String, Agent<C::Provider>>>,
    conversations: RwLock<BTreeMap<u64, Arc<Mutex<Conversation>>>>,
    next_conversation_id: AtomicU64,
    pub tools: ToolRegistry,
    steering: RwLock<BTreeMap<u64, watch::Sender<Option<String>>>>,
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
            agents: StdRwLock::new(BTreeMap::new()),
            ephemeral_agents: RwLock::new(BTreeMap::new()),
            conversations: RwLock::new(BTreeMap::new()),
            next_conversation_id: AtomicU64::new(1),
            tools,
            steering: RwLock::new(BTreeMap::new()),
        }
    }

    /// Access the persistence backend.
    pub fn storage(&self) -> &Arc<C::Storage> {
        &self.storage
    }
}
