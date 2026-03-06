//! OS hook — filesystem tools for agents.
//!
//! [`OsHook`] registers `read` and `write` tool schemas and provides
//! async dispatch methods backed by `tokio::fs`. Paths must be absolute.

use schemars::JsonSchema;
use serde::Deserialize;
use wcore::{ToolRegistry, model::Tool};

/// OS hook providing filesystem read/write tools.
pub struct OsHook;

impl OsHook {
    /// Dispatch a `read` tool call — read file at absolute path.
    pub async fn dispatch_read(&self, args: &str) -> String {
        let input: ReadInput = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        match tokio::fs::read_to_string(&input.path).await {
            Ok(content) => content,
            Err(e) => format!("read failed: {e}"),
        }
    }

    /// Dispatch a `write` tool call — write content to file at absolute path.
    pub async fn dispatch_write(&self, args: &str) -> String {
        let input: WriteInput = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        match tokio::fs::write(&input.path, &input.content).await {
            Ok(()) => format!("written: {}", input.path),
            Err(e) => format!("write failed: {e}"),
        }
    }
}

impl wcore::Hook for OsHook {
    fn on_register_tools(
        &self,
        registry: &mut ToolRegistry,
    ) -> impl std::future::Future<Output = ()> + Send {
        registry.insert(read_schema());
        registry.insert(write_schema());
        async {}
    }
}

#[derive(Deserialize, JsonSchema)]
struct ReadInput {
    /// Absolute path to the file to read
    path: String,
}

#[derive(Deserialize, JsonSchema)]
struct WriteInput {
    /// Absolute path to the file to write
    path: String,
    /// Content to write to the file
    content: String,
}

fn read_schema() -> Tool {
    Tool {
        name: "read".into(),
        description: "Read the contents of a file at an absolute path.".into(),
        parameters: schemars::schema_for!(ReadInput),
        strict: false,
    }
}

fn write_schema() -> Tool {
    Tool {
        name: "write".into(),
        description: "Write content to a file at an absolute path. Creates or overwrites the file."
            .into(),
        parameters: schemars::schema_for!(WriteInput),
        strict: false,
    }
}
