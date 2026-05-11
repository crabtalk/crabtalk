//! Bash policy — deny list with string containment.

use wcore::BashConfig;

/// Check a command against an explicit deny list. Returns the reason
/// if blocked. Used by `OsHook` after resolving the effective deny list
/// for the calling agent.
pub fn check_deny(deny: &[String], command: &str) -> Option<String> {
    deny.iter()
        .find(|d| command.contains(d.as_str()))
        .map(|d| format!("blocked: command contains '{d}'"))
}

/// Build a system prompt block describing the policy.
pub fn prompt_block(config: &BashConfig) -> Option<String> {
    if config.deny.is_empty() {
        return None;
    }
    Some(format!(
        "\n\n<bash-policy>\ndenied: {}\n</bash-policy>",
        config.deny.join(", ")
    ))
}
