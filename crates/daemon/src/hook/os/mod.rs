//! OS hook — filesystem and shell tools for agents.
//!
//! [`OsHook`] registers `read`, `write`, and `bash` tool schemas and provides
//! async dispatch methods. Access control is handled by the permission layer
//! in `dispatch_tool` — these tools dispatch directly without path validation.

use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::BTreeMap;
use wcore::{ToolRegistry, model::Tool};

/// OS hook providing filesystem and shell tools.
#[derive(Default)]
pub struct OsHook;

impl OsHook {
    /// Create a new `OsHook`.
    pub fn new() -> Self {
        Self
    }

    /// Dispatch a `read` tool call — read file at the given path.
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

    /// Dispatch a `write` tool call — write content to the given path.
    pub async fn dispatch_write(&self, args: &str) -> String {
        let input: WriteInput = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        let path = std::path::Path::new(&input.path);
        if let Some(parent) = path.parent()
            && let Err(e) = tokio::fs::create_dir_all(parent).await
        {
            return format!("write failed: {e}");
        }
        match tokio::fs::write(path, &input.content).await {
            Ok(()) => format!("written: {}", input.path),
            Err(e) => format!("write failed: {e}"),
        }
    }

    /// Dispatch a `bash` tool call — run a command directly.
    pub async fn dispatch_bash(&self, args: &str) -> String {
        let input: BashInput = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        let mut cmd = tokio::process::Command::new(&input.command);
        cmd.args(&input.args)
            .envs(&input.env)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return format!("bash failed: {e}"),
        };

        match tokio::time::timeout(std::time::Duration::from_secs(30), child.wait_with_output())
            .await
        {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.is_empty() {
                    stdout.into_owned()
                } else if stdout.is_empty() {
                    stderr.into_owned()
                } else {
                    format!("{stdout}\n{stderr}")
                }
            }
            Ok(Err(e)) => format!("bash failed: {e}"),
            Err(_) => "bash timed out after 30 seconds".to_owned(),
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
        registry.insert(bash_schema());
        async {}
    }
}

#[derive(Deserialize, JsonSchema)]
struct ReadInput {
    /// Path to the file to read.
    path: String,
}

#[derive(Deserialize, JsonSchema)]
struct WriteInput {
    /// Path to the file to write.
    path: String,
    /// Content to write to the file.
    content: String,
}

#[derive(Deserialize, JsonSchema)]
struct BashInput {
    /// Executable to run (e.g. `"ls"`, `"python3"`).
    command: String,
    /// Arguments to pass to the executable.
    #[serde(default)]
    args: Vec<String>,
    /// Environment variables to set for the process.
    #[serde(default)]
    env: BTreeMap<String, String>,
}

fn read_schema() -> Tool {
    Tool {
        name: "read".into(),
        description: "Read a file at the given path.".into(),
        parameters: schemars::schema_for!(ReadInput),
        strict: false,
    }
}

fn write_schema() -> Tool {
    Tool {
        name: "write".into(),
        description: "Write content to a file. Creates parent directories if needed.".into(),
        parameters: schemars::schema_for!(WriteInput),
        strict: false,
    }
}

fn bash_schema() -> Tool {
    Tool {
        name: "bash".into(),
        description: "Run a shell command.".into(),
        parameters: schemars::schema_for!(BashInput),
        strict: false,
    }
}
