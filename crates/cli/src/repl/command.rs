//! Slash command parsing, dispatch, and tab-completion for the REPL.

use anyhow::Result;
use rustyline::{
    Context,
    completion::{Completer, Pair},
    highlight::Highlighter,
};
use std::{borrow::Cow, path::Path};

pub const SLASH_COMMANDS: &[&str] = &["/help"];

/// Rustyline helper providing tab-completion and highlighting for slash commands.
#[derive(rustyline::Helper, rustyline::Hinter, rustyline::Validator)]
pub struct ReplHelper;

impl Highlighter for ReplHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        if !line.contains('/') {
            return Cow::Borrowed(line);
        }
        let mut out = String::with_capacity(line.len() + 32);
        let mut rest = line;
        while let Some(slash) = rest.find('/') {
            out.push_str(&rest[..slash]);
            rest = &rest[slash..];
            let end = rest[1..]
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '-')
                .map(|i| i + 1)
                .unwrap_or(rest.len());
            out.push_str(&console::style(&rest[..end]).dim().to_string());
            rest = &rest[end..];
        }
        out.push_str(rest);
        Cow::Owned(out)
    }

    fn highlight_char(
        &self,
        line: &str,
        _pos: usize,
        _kind: rustyline::highlight::CmdKind,
    ) -> bool {
        line.contains('/')
    }
}

impl Completer for ReplHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let prefix = &line[..pos];
        let Some(slash) = prefix.find('/') else {
            return Ok((0, vec![]));
        };
        let typed = &prefix[slash..]; // e.g. "/hel" or "/my-sk"

        let mut candidates: Vec<Pair> = SLASH_COMMANDS
            .iter()
            .filter(|cmd| cmd.starts_with(typed))
            .map(|cmd| Pair {
                display: cmd.to_string(),
                replacement: cmd.to_string(),
            })
            .collect();

        // Also complete skill names from disk.
        let skill_prefix = &typed[1..];
        if let Some(skills) = list_skill_names() {
            for name in skills {
                if name.starts_with(skill_prefix) {
                    let full = format!("/{name}");
                    candidates.push(Pair {
                        display: full.clone(),
                        replacement: full,
                    });
                }
            }
        }

        Ok((slash, candidates))
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
        "help" => {
            println!("Available commands:");
            println!("  /help    — show this help");
            println!("  /<skill> — run a skill");
        }
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
