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

pub use conversation::SwitchOutcome;

use crate::{Config, Conversation};
use memory::Memory;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, atomic::AtomicU64},
};
use tokio::sync::{Mutex, RwLock, watch};
use wcore::{Agent, ToolRegistry, model::Model};

/// Shared handle to the standalone memory store. Used by compaction to
/// write Archive entries and by session resume to pull their content
/// back as the replayed prefix.
pub type SharedMemory = Arc<parking_lot::RwLock<Memory>>;

#[derive(Clone)]
pub(super) struct ConvSlot {
    pub(super) agent: String,
    pub(super) created_by: String,
    pub(super) inner: Arc<Mutex<Conversation>>,
}

impl ConvSlot {
    pub(super) fn parts(&self) -> (String, String, Arc<Mutex<Conversation>>) {
        (
            self.agent.clone(),
            self.created_by.clone(),
            self.inner.clone(),
        )
    }
}

/// Per-(agent, sender) topic routing. `active = None` means the caller
/// is on a tmp chat (no topic). Tmp chats have their own `ConvSlot` but
/// are not tracked here.
#[derive(Default)]
pub(super) struct TopicRouter {
    pub(super) by_title: HashMap<String, u64>,
    pub(super) active: Option<String>,
    pub(super) tmp: Option<u64>,
}

impl TopicRouter {
    /// Resolve the conversation this router currently routes to:
    /// the active topic's conversation if one is set, otherwise the
    /// tmp conversation if one exists.
    pub(super) fn active_conversation(&self) -> Option<u64> {
        self.active
            .as_ref()
            .and_then(|t| self.by_title.get(t).copied())
            .or(self.tmp)
    }
}

/// The crabtalk runtime.
pub struct Runtime<C: Config> {
    pub model: Model<C::Provider>,
    pub env: Arc<C::Env>,
    storage: Arc<C::Storage>,
    memory: SharedMemory,
    agents: parking_lot::RwLock<BTreeMap<String, Agent<C::Provider>>>,
    ephemeral_agents: RwLock<BTreeMap<String, Agent<C::Provider>>>,
    conversations: RwLock<BTreeMap<u64, ConvSlot>>,
    pub(super) topics: RwLock<BTreeMap<(String, String), TopicRouter>>,
    next_conversation_id: AtomicU64,
    pub tools: ToolRegistry,
    steering: RwLock<BTreeMap<u64, watch::Sender<Option<String>>>>,
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
            ephemeral_agents: RwLock::new(BTreeMap::new()),
            conversations: RwLock::new(BTreeMap::new()),
            topics: RwLock::new(BTreeMap::new()),
            next_conversation_id: AtomicU64::new(1),
            tools,
            steering: RwLock::new(BTreeMap::new()),
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
}
