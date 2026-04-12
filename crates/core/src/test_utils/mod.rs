//! Shared test scaffolding for crabtalk-core and downstream crates.
//!
//! [`InMemoryStorage`] for a pluggable [`Storage`] without a filesystem,
//! [`test_provider`] for a scripted [`Provider`], and [`test_schema`]
//! for a minimal [`Tool`] schema.

use crabllm_core::{FunctionDef, Tool, ToolType};

mod mem;
pub mod test_provider;

pub use mem::InMemoryStorage;

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
