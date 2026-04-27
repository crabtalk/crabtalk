//! Slash command dispatch and candidate collection for the REPL.

use anyhow::Result;
use sdk::{COMMANDS, Command, parse_command};

/// Collect matching `/command` and `/skill` names for the typed prefix.
pub fn collect_candidates(line: &str, pos: usize, skill_names: &[String]) -> Vec<String> {
    let prefix = &line[..pos];
    let Some(slash) = prefix.find('/') else {
        return Vec::new();
    };
    let typed = &prefix[slash..];

    let mut candidates: Vec<String> = COMMANDS
        .iter()
        .filter(|cmd| cmd.starts_with(typed))
        .map(|cmd| cmd.to_string())
        .collect();

    let skill_prefix = &typed[1..];
    for name in skill_names {
        if name.starts_with(skill_prefix) {
            candidates.push(format!("/{name}"));
        }
    }

    candidates
}

/// Result of handling a slash command in the TUI.
pub enum SlashResult {
    /// The line was handled locally (e.g. printed help).
    Handled,
    /// Not a slash command — send the line as-is.
    NotSlash,
    /// A slash command to forward to the daemon.
    Forward(String),
    /// Exit the REPL.
    Exit,
    /// Clear context and start a new conversation.
    Clear,
    /// Open the conversation console.
    Resume,
}

/// Map a chat-input line to a TUI dispatch action.
pub async fn handle_slash(line: &str) -> Result<SlashResult> {
    match parse_command(line) {
        None => Ok(SlashResult::NotSlash),
        Some(Command::Clear) => Ok(SlashResult::Clear),
        Some(Command::Exit) => Ok(SlashResult::Exit),
        Some(Command::Resume) => Ok(SlashResult::Resume),
        Some(Command::Forward(line)) => Ok(SlashResult::Forward(line)),
        Some(Command::Help) => {
            println!("Available commands:");
            println!("  /clear   — start a new conversation");
            println!("  /exit    — exit the REPL");
            println!("  /help    — show this help");
            println!("  /resume  — open conversation console");
            println!("  /<skill> — run a skill");
            Ok(SlashResult::Handled)
        }
    }
}
