//! Hook trait — type-level runtime configuration.
//!
//! Hook is a pure trait with no `&self` parameter. It tells the Runtime
//! which [`Memory`] backend and LLM provider to use, and what prompts
//! to send for automatic compaction and memory flush.

use agent::{InMemory, Memory};
use llm::{General, LLM, NoopProvider};

/// Type-level runtime configuration.
///
/// Determines the LLM provider, memory backend, and compaction/flush prompts.
/// No instances are created — methods are called as `H::compact()`.
pub trait Hook {
    /// The LLM provider for this hook.
    type Provider: LLM + Send + Sync;

    /// The memory backend for this hook.
    type Memory: Memory;

    /// Compaction prompt sent to the LLM to summarize conversation history
    /// when context approaches the limit. Return empty string to disable.
    fn compact() -> &'static str;

    /// Memory flush prompt sent before compaction to extract durable facts
    /// into memory via the "remember" tool. Return empty string to skip.
    fn flush() -> &'static str;

    /// Context window limit in tokens. Override for model-specific defaults.
    fn context_limit(config: &General) -> usize {
        config.context_limit.unwrap_or(64_000)
    }
}

/// Default compaction prompt.
pub const DEFAULT_COMPACT_PROMPT: &str = include_str!("../prompts/compact.md");

/// Default memory flush prompt.
pub const DEFAULT_FLUSH_PROMPT: &str = include_str!("../prompts/flush.md");

impl Hook for () {
    type Provider = NoopProvider;
    type Memory = InMemory;

    fn compact() -> &'static str {
        DEFAULT_COMPACT_PROMPT
    }

    fn flush() -> &'static str {
        DEFAULT_FLUSH_PROMPT
    }
}
