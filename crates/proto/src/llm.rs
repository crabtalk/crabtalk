//! Conversions between the wire messages and the LLM types they carry.
//!
//! Both sides are foreign to `crabtalk-schema`, so the impls live here — with
//! the generated types, where the orphan rule allows them.
#![cfg(feature = "llm")]

use crate::TokenUsage;
use crabllm_core::Usage;

impl From<&Usage> for TokenUsage {
    fn from(u: &Usage) -> Self {
        Self {
            prompt_tokens: u.prompt_tokens(),
            completion_tokens: u.completion_tokens(),
            total_tokens: u.total_tokens(),
            cache_hit_tokens: (u.cache_read_tokens > 0).then_some(u.cache_read_tokens),
            cache_miss_tokens: (u.cache_write_tokens > 0).then_some(u.cache_write_tokens),
            reasoning_tokens: (u.reasoning_tokens > 0).then_some(u.reasoning_tokens),
        }
    }
}
