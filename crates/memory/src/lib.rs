//! Structured knowledge memory for LLM agents.
//!
//! Memory is **not chat history**. It is structured knowledge — extracted
//! facts, user preferences, agent persona — that gets compiled into the
//! system prompt before each LLM request.
//!
//! # Architecture
//!
//! The [`Memory`] trait is fully synchronous. Persistence (loading from
//! and saving to a database) is the application's concern. This mirrors
//! how `Chat` owns `Vec<Message>` in-process and leaves DB persistence
//! to the consumer.
//!
//! Integration with agents uses the decorator pattern: [`WithMemory<A, M>`]
//! wraps any [`Agent`] and injects compiled memory into the system prompt.
//! No changes to the core `Agent` or `Chat` types are required.
//!
//! # Example
//!
//! ```rust,ignore
//! use cydonia_memory::{InMemory, Memory, WithMemory};
//!
//! let mut memory = InMemory::new();
//! memory.set("user", "Prefers short answers. Based in Singapore.");
//! memory.set("persona", "You are a cautious trader.");
//!
//! let agent = WithMemory::new(PerpAgent::new(pool, &req), memory);
//! let chat = Chat::new(config, provider, agent, messages);
//! ```

pub use agent::{MemoryLayer, WithMemory};
pub use store::InMemory;

mod agent;
mod store;

/// Structured knowledge memory for LLM agents.
///
/// Implementations store named key-value pairs that get compiled
/// into the system prompt via [`compile()`](Memory::compile).
///
/// The trait is fully synchronous — no async, no I/O. Persistence
/// is the application's concern.
pub trait Memory: Clone + Send + Sync {
    /// Get the value for a key.
    fn get(&self, key: &str) -> Option<&str>;

    /// Get all key-value pairs.
    fn entries(&self) -> &[(String, String)];

    /// Set (upsert) a key-value pair. Returns the previous value if the key existed.
    fn set(&mut self, key: impl Into<String>, value: impl Into<String>) -> Option<String>;

    /// Remove a key. Returns the removed value if it existed.
    fn remove(&mut self, key: &str) -> Option<String>;

    /// Compile all entries into a string for system prompt injection.
    ///
    /// The default implementation produces XML-style blocks:
    ///
    /// ```text
    /// <memory>
    /// <user>
    /// Prefers short answers.
    /// </user>
    /// </memory>
    /// ```
    fn compile(&self) -> String {
        let entries = self.entries();
        if entries.is_empty() {
            return String::new();
        }

        let mut out = String::from("<memory>\n");
        for (key, value) in entries {
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
}
