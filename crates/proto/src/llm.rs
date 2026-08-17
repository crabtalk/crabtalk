//! Conversions between the wire messages and the LLM types they carry.
//!
//! Both sides are foreign to `crabtalk-core`, so the impls live here — with
//! the generated types, where the orphan rule allows them.
#![cfg(feature = "llm")]

use crate::{TokenUsage, ToolDef};
use crabllm_core::{FunctionDef, Tool, ToolType, Usage};

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

/// A client tool as declared on the wire. The schema travels as a JSON
/// string, so an unparsable one degrades to "no parameters" rather than
/// failing the whole stream.
impl From<ToolDef> for Tool {
    fn from(def: ToolDef) -> Self {
        Self {
            kind: ToolType::Function,
            function: FunctionDef {
                name: def.name,
                description: (!def.description.is_empty()).then_some(def.description),
                parameters: (!def.parameters_schema.is_empty())
                    .then(|| serde_json::from_str(&def.parameters_schema).ok())
                    .flatten(),
            },
            strict: None,
            cache_control: None,
        }
    }
}
