//! Structured knowledge memory for LLM agents.
//!
//! Memory is **not chat history**. It is structured knowledge — extracted
//! facts, user preferences, agent persona — that gets compiled into the
//! system prompt.
//!
//! # Usage
//!
//! ```rust,ignore
//! use walrus_core::{Agent, InMemory, Memory, with_memory};
//!
//! let memory = InMemory::new();
//! memory.set("user", "Prefers short answers.");
//!
//! let agent = Agent::new("anto").system_prompt("You are helpful.");
//! let agent = with_memory(agent, &memory);
//! ```

pub use embedder::{Embedder, NoEmbedder};
pub use mem::InMemory;

use crate::Agent;
use compact_str::CompactString;
use serde_json::Value;
use std::future::Future;
mod embedder;
mod mem;

/// A structured memory entry with metadata and optional embedding.
#[derive(Debug, Clone, Default)]
pub struct MemoryEntry {
    /// Entry key (identity string).
    pub key: CompactString,
    /// Entry value (unbounded content).
    pub value: String,
    /// Optional structured metadata (JSON).
    pub metadata: Option<Value>,
    /// Unix timestamp when the entry was created.
    pub created_at: u64,
    /// Unix timestamp when the entry was last accessed.
    pub accessed_at: u64,
    /// Number of times the entry has been accessed.
    pub access_count: u32,
    /// Optional embedding vector for semantic search.
    pub embedding: Option<Vec<f32>>,
}

/// Options controlling memory recall behavior.
#[derive(Debug, Clone, Default)]
pub struct RecallOptions {
    /// Maximum number of results (0 = implementation default).
    pub limit: usize,
    /// Filter by creation time range (start, end) in unix seconds.
    pub time_range: Option<(u64, u64)>,
    /// Minimum relevance score threshold (0.0–1.0).
    pub relevance_threshold: Option<f32>,
}

/// Structured knowledge memory for LLM agents.
///
/// Implementations store named key-value pairs that get compiled
/// into the system prompt via [`compile()`](Memory::compile).
///
/// Uses `&self` for all methods — implementations must handle
/// interior mutability (e.g. via `Mutex`).
pub trait Memory: Send + Sync {
    /// Get the value for a key (owned).
    fn get(&self, key: &str) -> Option<String>;

    /// Get all key-value pairs (owned).
    fn entries(&self) -> Vec<(String, String)>;

    /// Set (upsert) a key-value pair. Returns the previous value if the key existed.
    fn set(&self, key: impl Into<String>, value: impl Into<String>) -> Option<String>;

    /// Remove a key. Returns the removed value if it existed.
    fn remove(&self, key: &str) -> Option<String>;

    /// Compile all entries into a string for system prompt injection.
    fn compile(&self) -> String {
        let entries = self.entries();
        if entries.is_empty() {
            return String::new();
        }

        let mut out = String::from("<memory>\n");
        for (key, value) in &entries {
            out.push_str(&format!("<{key}>\n"));
            out.push_str(value);
            if !value.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&format!("</{key}>\n"));
        }
        out.push_str("</memory>");
        out
    }

    /// Store a key-value pair (async). Default delegates to `set`.
    fn store(
        &self,
        key: impl Into<String> + Send,
        value: impl Into<String> + Send,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        self.set(key, value);
        async { Ok(()) }
    }

    /// Search for relevant entries (async). Default returns empty.
    fn recall(
        &self,
        _query: &str,
        _options: RecallOptions,
    ) -> impl Future<Output = anyhow::Result<Vec<MemoryEntry>>> + Send {
        async { Ok(Vec::new()) }
    }

    /// Compile relevant entries for a query (async). Default delegates to `compile`.
    fn compile_relevant(&self, _query: &str) -> impl Future<Output = String> + Send {
        let compiled = self.compile();
        async move { compiled }
    }
}

/// Apply memory to an agent — appends compiled memory to the system prompt.
pub fn with_memory(mut agent: Agent, memory: &impl Memory) -> Agent {
    let compiled = memory.compile();
    if !compiled.is_empty() {
        agent.system_prompt = format!("{}\n\n{compiled}", agent.system_prompt);
    }
    agent
}
