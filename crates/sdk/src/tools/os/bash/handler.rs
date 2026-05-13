//! Bash tool handler.

use super::Bash;
use crate::tools::os::OsHook;

impl OsHook {
    pub(in crate::tools::os) async fn handle_bash(&self, args: &str) -> Result<String, String> {
        let input: Bash =
            serde_json::from_str(args).map_err(|e| format!("invalid arguments: {e}"))?;

        let cwd = self.effective_cwd();

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
