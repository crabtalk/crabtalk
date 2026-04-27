//! TUI dispatch over [`sdk::Command`].

use anyhow::Result;
use sdk::{Command, parse_command};

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
