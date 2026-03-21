//! Tool schemas and input types for OS tools.

use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::BTreeMap;
use wcore::{
    agent::{AsTool, ToolDescription},
    model::Tool,
};

#[derive(Deserialize, JsonSchema)]
pub(crate) struct Bash {
    /// Executable to run (e.g. `"ls"`, `"python3"`).
    pub command: String,
    /// Arguments to pass to the executable.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables to set for the process.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl ToolDescription for Bash {
    const DESCRIPTION: &'static str = "Run a shell command.";
}

pub(crate) fn tools() -> Vec<Tool> {
    vec![Bash::as_tool()]
}

impl crate::hook::DaemonHook {
    /// Dispatch a `bash` tool call — run a command directly.
    ///
    /// Returns structured JSON: `{"stdout":"...","stderr":"...","exit_code":N}`.
    pub(crate) async fn dispatch_bash(&self, args: &str, session_id: Option<u64>) -> String {
        let input: Bash = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        let session_cwd = if let Some(id) = session_id {
            self.session_cwds.lock().await.get(&id).cloned()
        } else {
            None
        };
        let cwd = session_cwd.as_deref().unwrap_or(&self.cwd);
        let mut cmd = tokio::process::Command::new(&input.command);
        cmd.args(&input.args)
            .envs(&input.env)
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return serde_json::json!({
                    "stdout": "",
                    "stderr": format!("bash failed: {e}"),
                    "exit_code": -1
                })
                .to_string();
            }
        };

        match tokio::time::timeout(std::time::Duration::from_secs(30), child.wait_with_output())
            .await
        {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let exit_code = output.status.code().unwrap_or(-1);
                serde_json::json!({
                    "stdout": stdout.as_ref(),
                    "stderr": stderr.as_ref(),
                    "exit_code": exit_code
                })
                .to_string()
            }
            Ok(Err(e)) => serde_json::json!({
                "stdout": "",
                "stderr": format!("bash failed: {e}"),
                "exit_code": -1
            })
            .to_string(),
            Err(_) => serde_json::json!({
                "stdout": "",
                "stderr": "bash timed out after 30 seconds",
                "exit_code": -1
            })
            .to_string(),
        }
    }
}
