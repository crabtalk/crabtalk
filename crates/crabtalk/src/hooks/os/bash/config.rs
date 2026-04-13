//! Bash policy — allow/ask/deny command classification.

use serde::Deserialize;
use std::path::Path;

/// Result of a policy check on a single command.
pub(in crate::hooks::os) enum Verdict {
    /// Command matches an allow pattern — execute immediately.
    Allow,
    /// Command matches a deny pattern — reject, no override.
    Deny(String),
    /// Command matches an ask pattern, or doesn't match any allow pattern —
    /// prompt the user for approval.
    Ask(String),
}

/// Bash policy loaded from `~/.crabtalk/config/bash.toml`.
#[derive(Deserialize, Default)]
pub struct BashConfig {
    /// Command prefix patterns that are allowed (e.g. `"cargo *"`, `"git *"`).
    #[serde(default)]
    pub allow: Vec<String>,
    /// Command prefix patterns that require interactive approval (e.g. `"git push *"`).
    #[serde(default)]
    pub ask: Vec<String>,
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
    fn check_command(&self, command: &str) -> Verdict {
        // Deny always wins.
        if self.deny.iter().any(|p| matches_pattern(p, command)) {
            return Verdict::Deny(format!("denied by policy: {command}"));
        }

        // Allow matches → execute.
        if self.allow.iter().any(|p| matches_pattern(p, command)) {
            return Verdict::Allow;
        }

        // Ask matches → prompt.
        if self.ask.iter().any(|p| matches_pattern(p, command)) {
            return Verdict::Ask(format!("requires approval: {command}"));
        }

        // No allow list configured → allow everything (backwards compat).
        if self.allow.is_empty() && self.ask.is_empty() {
            return Verdict::Allow;
        }

        // Has rules but no match → needs approval.
        Verdict::Ask(format!("not in allow list: {command}"))
    }

    /// Check a (possibly compound) command. Returns the most restrictive
    /// verdict across all subcommands: Deny > Ask > Allow.
    pub(in crate::hooks::os) fn check(&self, command: &str) -> Verdict {
        let mut needs_ask = None;
        for sub in split_compound(command) {
            match self.check_command(sub) {
                Verdict::Deny(reason) => return Verdict::Deny(reason),
                Verdict::Ask(reason) => needs_ask = Some(reason),
                Verdict::Allow => {}
            }
        }
        match needs_ask {
            Some(reason) => Verdict::Ask(reason),
            None => Verdict::Allow,
        }
    }

    /// Build a system prompt block describing the policy.
    pub fn prompt_block(&self) -> Option<String> {
        if self.allow.is_empty() && self.ask.is_empty() && self.deny.is_empty() {
            return None;
        }
        let mut block = String::from("\n\n<bash-policy>\n");
        if !self.allow.is_empty() {
            block.push_str(&format!("allowed: {}\n", self.allow.join(", ")));
        }
        if !self.ask.is_empty() {
            block.push_str(&format!("requires approval: {}\n", self.ask.join(", ")));
        }
        if !self.deny.is_empty() {
            block.push_str(&format!("denied: {}\n", self.deny.join(", ")));
        }
        if !self.allow.is_empty() || !self.ask.is_empty() {
            block.push_str(
                "Commands not matching the allow or ask list require interactive approval.\n",
            );
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
        // Two-character operators: && and ||
        if (bytes[i] == b'&' || bytes[i] == b'|') && i + 1 < bytes.len() && bytes[i + 1] == bytes[i]
        {
            parts.push(command[start..i].trim());
            i += 2;
            start = i;
        } else if bytes[i] == b'|' || bytes[i] == b';' || bytes[i] == b'\n' {
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
