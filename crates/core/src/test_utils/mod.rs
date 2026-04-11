//! Shared test scaffolding for crabtalk-core and downstream crates.
//!
//! All types and helpers gated on the `test-utils` feature live here
//! — [`TestHook`] for a no-op [`Hook`] + [`ToolDispatcher`],
//! [`InMemoryStorage`] for a pluggable [`Storage`] without a
//! filesystem, [`test_provider`] for a scripted [`Provider`], and
//! [`test_schema`] for a minimal [`Tool`] schema.

use crate::{
    agent::tool::{ToolDispatcher, ToolFuture},
    runtime::hook::Hook,
};
use crabllm_core::{FunctionDef, Tool, ToolType};

mod mem;
pub mod test_provider;

pub use mem::InMemoryStorage;

/// Trivial [`Hook`] for tests that don't need lifecycle customization.
#[derive(Default, Clone)]
pub struct TestHook;

impl Hook for TestHook {}

impl ToolDispatcher for TestHook {
    fn dispatch<'a>(
        &'a self,
        name: &'a str,
        _args: &'a str,
        _agent: &'a str,
        _sender: &'a str,
        _conversation_id: Option<u64>,
    ) -> ToolFuture<'a> {
        let name = name.to_owned();
        Box::pin(async move { Err(format!("TestHook: tool '{name}' not dispatched")) })
    }
}

/// Create a minimal tool schema for testing.
pub fn test_schema(name: &str) -> Tool {
    Tool {
        kind: ToolType::Function,
        function: FunctionDef {
            name: name.to_owned(),
            description: None,
            parameters: None,
        },
        strict: None,
    }
}
