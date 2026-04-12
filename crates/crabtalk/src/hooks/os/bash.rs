//! Bash tool schema, config, and handler.

use super::OsHook;
use schemars::JsonSchema;
use serde::Deserialize;
use std::{collections::BTreeMap, path::Path};
use wcore::{ToolDispatch, agent::ToolDescription};

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

// ── Config ──────────────────────────────────────────────────────────

/// Bash policy loaded from `~/.crabtalk/config/bash.toml`.
#[derive(Deserialize, Default)]
pub struct BashConfig {
    /// Command prefix patterns that are allowed (e.g. `"cargo *"`, `"git *"`).
    #[serde(default)]
    pub allow: Vec<String>,
    /// Command prefix patterns that are denied (e.g. `"rm -rf *"`, `"sudo *"`).
    #[serde(default)]
    pub deny: Vec<String>,
}

impl BashConfig {
    /// Load from `{config_dir}/config/bash.toml`. Returns default (no
    /// restrictions) if the file doesn't exist.
    pub fn load(config_dir: &Path) -> Self {
        let path = config_dir.join("config").join("bash.toml");
        match std::fs::read_to_string(&path) {
            Ok(content) => match toml::from_str(&content) {
                Ok(config) => config,
                Err(e) => {
                    tracing::warn!("invalid bash config at {}: {e}", path.display());
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    /// Check a single command against the policy.
    ///
    /// Returns `Ok(())` if allowed, `Err(reason)` if denied.
    fn check_command(&self, command: &str) -> Result<(), String> {
        // Deny always wins.
        for pattern in &self.deny {
            if matches_pattern(pattern, command) {
                return Err(format!("denied by policy: {command}"));
            }
        }

        // If allow list is non-empty, command must match at least one.
        if !self.allow.is_empty() {
            for pattern in &self.allow {
                if matches_pattern(pattern, command) {
                    return Ok(());
                }
            }
            return Err(format!("not in allow list: {command}"));
        }

        // No allow list configured → allow everything (backwards compat).
        Ok(())
    }

    /// Check a (possibly compound) command against the policy.
    pub fn check(&self, command: &str) -> Result<(), String> {
        for sub in split_compound(command) {
            self.check_command(sub)?;
        }
        Ok(())
    }

    /// Build a system prompt block describing the policy.
    pub fn prompt_block(&self) -> Option<String> {
        if self.allow.is_empty() && self.deny.is_empty() {
            return None;
        }
        let mut block = String::from("\n\n<bash-policy>\n");
        if !self.allow.is_empty() {
            block.push_str(&format!("allowed: {}\n", self.allow.join(", ")));
        }
        if !self.deny.is_empty() {
            block.push_str(&format!("denied: {}\n", self.deny.join(", ")));
        }
        if !self.allow.is_empty() {
            block.push_str("Commands not matching the allow list will be rejected.\n");
        }
        block.push_str("</bash-policy>");
        Some(block)
    }
}

// ── Pattern matching ────────────────────────────────────────────────

/// Match a pattern against a command. CC-style raw string prefix matching.
///
/// - `"cargo *"` → command starts with `"cargo "` or equals `"cargo"`
/// - `"ls"` → exact match only
/// - `"*"` → matches everything
fn matches_pattern(pattern: &str, command: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        command.starts_with(prefix) || command == prefix.trim_end()
    } else {
        command == pattern
    }
}

/// Split a compound command on shell operators (`&&`, `||`, `|`, `;`).
/// Each subcommand is trimmed. Empty parts are dropped.
fn split_compound(command: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let bytes = command.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' && i + 1 < bytes.len() && bytes[i + 1] == b'&' {
            parts.push(command[start..i].trim());
            i += 2;
            start = i;
        } else if bytes[i] == b'|' && i + 1 < bytes.len() && bytes[i + 1] == b'|' {
            parts.push(command[start..i].trim());
            i += 2;
            start = i;
        } else if bytes[i] == b'|' {
            parts.push(command[start..i].trim());
            i += 1;
            start = i;
        } else if bytes[i] == b';' || bytes[i] == b'\n' {
            parts.push(command[start..i].trim());
            i += 1;
            start = i;
        } else {
            i += 1;
        }
    }
    if start < bytes.len() {
        parts.push(command[start..].trim());
    }
    parts.into_iter().filter(|s| !s.is_empty()).collect()
}

// ── Handler ─────────────────────────────────────────────────────────

impl OsHook {
    pub(super) async fn handle_bash(&self, call: ToolDispatch) -> Result<String, String> {
        let input: Bash =
            serde_json::from_str(&call.args).map_err(|e| format!("invalid arguments: {e}"))?;

        // Policy check.
        self.bash_config.check(&input.command)?;

        let cwd = self.effective_cwd(call.conversation_id);

        let mut cmd = tokio::process::Command::new("bash");
        cmd.args(["-c", &input.command])
            .envs(&input.env)
            .current_dir(&cwd)
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
