//! Tool schemas and input types for OS tools.

use crate::{Env, host::Host};
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::BTreeMap;
use wcore::{
    agent::{AsTool, ToolDescription},
    model::Tool,
};

#[derive(Deserialize, JsonSchema)]
pub struct Bash {
    /// Shell command to run (e.g. `"ls -la"`, `"cat foo.txt | grep bar"`).
    pub command: String,
    /// Environment variables to set for the process.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl ToolDescription for Bash {
    const DESCRIPTION: &'static str = "Run a shell command.";
}

pub fn tools() -> Vec<Tool> {
    vec![Bash::as_tool()]
}

impl<H: Host> Env<H> {
    /// Dispatch a `bash` tool call — run a command directly.
    ///
    /// A non-zero exit code is *not* an `Err` — the shell ran successfully
    /// and the JSON payload describes the outcome. Only dispatcher-level
    /// failures (spawn error, wait error, timeout) become `Err`.
    pub async fn dispatch_bash(
        &self,
        args: &str,
        conversation_id: Option<u64>,
    ) -> Result<String, String> {
        let input: Bash =
            serde_json::from_str(args).map_err(|e| format!("invalid arguments: {e}"))?;
        let conversation_cwd = if let Some(id) = conversation_id {
            self.host.conversation_cwd(id)
        } else {
            None
        };
        let cwd = conversation_cwd.as_deref().unwrap_or(&self.cwd);

        let mut cmd = tokio::process::Command::new("bash");
        cmd.args(["-c", &input.command])
            .envs(&input.env)
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let child = cmd.spawn().map_err(|e| {
            serde_json::json!({
                "stdout": "",
                "stderr": format!("bash failed: {e}"),
                "exit_code": -1
            })
            .to_string()
        })?;

        match tokio::time::timeout(std::time::Duration::from_secs(30), child.wait_with_output())
            .await
        {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let exit_code = output.status.code().unwrap_or(-1);
                Ok(serde_json::json!({
                    "stdout": stdout.as_ref(),
                    "stderr": stderr.as_ref(),
                    "exit_code": exit_code
                })
                .to_string())
            }
            Ok(Err(e)) => Err(serde_json::json!({
                "stdout": "",
                "stderr": format!("bash failed: {e}"),
                "exit_code": -1
            })
            .to_string()),
            Err(_) => Err(serde_json::json!({
                "stdout": "",
                "stderr": "bash timed out after 30 seconds",
                "exit_code": -1
            })
            .to_string()),
        }
    }
}
