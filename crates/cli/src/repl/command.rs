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
///
/// Scans `local/skills/` plus any cached repo skill dirs from packages.
fn list_skill_names() -> Option<Vec<String>> {
    let config_dir = &*wcore::paths::CONFIG_DIR;
    let mut all_names = std::collections::BTreeSet::new();

    // Local skills.
    let local_skills = config_dir.join(wcore::paths::SKILLS_DIR);
    if let Some(names) = list_skill_dirs(&local_skills) {
        all_names.extend(names);
    }

    // Package skills from cached repos.
    let packages_dir = config_dir.join(wcore::paths::PACKAGES_DIR);
    if let Ok(scopes) = std::fs::read_dir(&packages_dir) {
        for scope in scopes.flatten() {
            let scope_path = scope.path();
            if !scope_path.is_dir() {
                continue;
            }
            if let Ok(packages) = std::fs::read_dir(&scope_path) {
                for pkg in packages.flatten() {
                    let pkg_path = pkg.path();
                    if pkg_path.extension().is_some_and(|e| e == "toml") {
                        // Try to read repository from manifest to find cached repo.
                        if let Ok(content) = std::fs::read_to_string(&pkg_path)
                            && let Some(repo) = extract_repository(&content)
                        {
                            let slug = repo_slug(&repo);
                            let skills = config_dir
                                .join(".cache")
                                .join("repos")
                                .join(&slug)
                                .join("skills");
                            if let Some(names) = list_skill_dirs(&skills) {
                                all_names.extend(names);
                            }
                        }
                    }
                }
            }
        }
    }

    // External tool skill directories.
    if let Some(home) = dirs::home_dir() {
        for dir in [".claude/skills", ".codex/skills", ".openclaw/skills"] {
            if let Some(names) = list_skill_dirs(&home.join(dir)) {
                all_names.extend(names);
            }
        }
    }

    if all_names.is_empty() {
        None
    } else {
        Some(all_names.into_iter().collect())
    }
}

/// Extract the repository URL from a manifest TOML string.
fn extract_repository(toml_content: &str) -> Option<String> {
    let doc: toml_edit::DocumentMut = toml_content.parse().ok()?;
    doc.get("package")?
        .get("repository")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_owned())
}

/// Convert a repo URL to a filesystem-safe slug.
fn repo_slug(url: &str) -> String {
    wcore::repo_slug(url)
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
