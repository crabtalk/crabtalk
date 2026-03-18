//! Slash command parsing, dispatch, and tab-completion for the REPL.

use anyhow::Result;
use rustyline::{
    Context,
    completion::{Completer, Pair},
};
use std::path::Path;

pub const SLASH_COMMANDS: &[&str] = &["/help", "/switch"];

/// Rustyline helper providing tab-completion for slash commands.
#[derive(rustyline::Helper, rustyline::Hinter, rustyline::Highlighter, rustyline::Validator)]
pub struct ReplHelper;

impl Completer for ReplHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let prefix = &line[..pos];
        if !prefix.starts_with('/') {
            return Ok((0, vec![]));
        }
        let mut candidates: Vec<Pair> = SLASH_COMMANDS
            .iter()
            .filter(|cmd| cmd.starts_with(prefix))
            .map(|cmd| Pair {
                display: cmd.to_string(),
                replacement: cmd.to_string(),
            })
            .collect();

        // Also complete skill names from disk.
        let slash_prefix = &prefix[1..];
        if let Some(skills) = list_skill_names() {
            for name in skills {
                if name.starts_with(slash_prefix) {
                    let full = format!("/{name}");
                    candidates.push(Pair {
                        display: full.clone(),
                        replacement: full,
                    });
                }
            }
        }

        Ok((0, candidates))
    }
}

/// Result of handling a slash command.
pub enum SlashResult {
    /// The line was handled locally (printed help, switched agent, etc.).
    Handled,
    /// Not a slash command — send the line as-is.
    NotSlash,
    /// A slash command to forward to the daemon (e.g. `/skill args`).
    Forward(String),
}

/// Dispatch a slash command.
pub async fn handle_slash(agent: &mut String, line: &str) -> Result<SlashResult> {
    if !line.starts_with('/') {
        return Ok(SlashResult::NotSlash);
    }
    let rest = &line[1..];
    let (cmd, _arg) = match rest.find(' ') {
        Some(pos) => (&rest[..pos], Some(rest[pos + 1..].trim())),
        None => (rest, None),
    };
    match cmd {
        "help" => {
            println!("Available commands:");
            println!("  /help          — show this help");
            println!("  /switch <name> — switch active agent");
            println!("  /<skill>       — run a skill");
        }
        "switch" => match _arg {
            Some(name) if !name.is_empty() => {
                *agent = name.to_owned();
                println!("Switched to agent '{name}'.");
            }
            _ => println!("Usage: /switch <agent-name>"),
        },
        _ => {
            // Forward to daemon for skill resolution.
            return Ok(SlashResult::Forward(line.to_owned()));
        }
    }
    Ok(SlashResult::Handled)
}

/// List skill directory names for tab completion.
fn list_skill_names() -> Option<Vec<String>> {
    let skills_dir = wcore::paths::CONFIG_DIR.join(wcore::paths::SKILLS_DIR);
    list_skill_dirs(&skills_dir)
}

/// Read skill subdirectory names that contain a SKILL.md file.
fn list_skill_dirs(dir: &Path) -> Option<Vec<String>> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut names = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir()
            && path.join("SKILL.md").exists()
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
        {
            names.push(name.to_owned());
        }
    }
    names.sort();
    Some(names)
}
