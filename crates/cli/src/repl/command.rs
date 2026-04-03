//! Slash command parsing and candidate collection for the REPL.

use anyhow::Result;

pub const SLASH_COMMANDS: &[&str] = &["/clear", "/exit", "/help", "/resume"];

/// Collect matching `/command` and `/skill` names for the typed prefix.
pub fn collect_candidates(line: &str, pos: usize, skill_names: &[String]) -> Vec<String> {
    let prefix = &line[..pos];
    let Some(slash) = prefix.find('/') else {
        return Vec::new();
    };
    let typed = &prefix[slash..];

    let mut candidates: Vec<String> = SLASH_COMMANDS
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

/// Result of handling a slash command.
pub enum SlashResult {
    /// The line was handled locally (printed help, switched agent, etc.).
    Handled,
    /// Not a slash command — send the line as-is.
    NotSlash,
    /// A slash command to forward to the daemon (e.g. `/skill args`).
    Forward(String),
    /// Exit the REPL.
    Exit,
    /// Clear context and start a new conversation.
    Clear,
    /// Open the conversation console.
    Resume,
}

/// Dispatch a slash command.
pub async fn handle_slash(line: &str) -> Result<SlashResult> {
    if !line.starts_with('/') {
        return Ok(SlashResult::NotSlash);
    }
    let rest = &line[1..];
    let (cmd, _arg) = match rest.find(' ') {
        Some(pos) => (&rest[..pos], Some(rest[pos + 1..].trim())),
        None => (rest, None),
    };
    match cmd {
        "clear" => return Ok(SlashResult::Clear),
        "exit" => return Ok(SlashResult::Exit),
        "help" => {
            println!("Available commands:");
            println!("  /clear   — start a new conversation");
            println!("  /exit    — exit the REPL");
            println!("  /help    — show this help");
            println!("  /resume  — open conversation console");
            println!("  /<skill> — run a skill");
        }
        "resume" => return Ok(SlashResult::Resume),
        _ => {
            // Forward to daemon for skill resolution.
            return Ok(SlashResult::Forward(line.to_owned()));
        }
    }
    Ok(SlashResult::Handled)
}
