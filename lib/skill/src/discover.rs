//! Finding skills on disk.
//!
//! A skill is a directory holding a `SKILL.md`, so discovery is the same
//! everywhere regardless of where sessions and agents are kept. Roots are
//! searched in order and the first one wins a name collision.

use crate::Skill;
use anyhow::Result;
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};
use tokio::fs;

/// Every skill across `roots`, first root winning on name collisions.
pub async fn list(roots: &[PathBuf]) -> Result<Vec<Skill>> {
    let mut skills = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        let mut entries = match fs::read_dir(root).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) if !n.starts_with('.') => n.to_owned(),
                _ => continue,
            };
            if seen.contains(&name) {
                continue;
            }
            let skill_path = path.join("SKILL.md");
            if !skill_path.exists() {
                continue;
            }
            let content = match fs::read_to_string(&skill_path).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("failed to read {}: {e}", skill_path.display());
                    continue;
                }
            };
            match content.parse::<Skill>() {
                Ok(skill) => {
                    seen.insert(name);
                    skills.push(skill);
                }
                Err(e) => tracing::warn!("failed to parse {}: {e}", skill_path.display()),
            }
        }
    }
    Ok(skills)
}

/// One skill by name, searching `roots` in order.
pub async fn load(roots: &[PathBuf], name: &str) -> Result<Option<Skill>> {
    for root in roots {
        let skill_path = root.join(name).join("SKILL.md");
        if !skill_path.exists() {
            continue;
        }
        let content = fs::read_to_string(&skill_path).await?;
        return Ok(Some(content.parse()?));
    }
    Ok(None)
}

/// Check for skill name conflicts across multiple skill directories.
///
/// First directory wins, matching [`list`].
pub fn check_conflicts(skill_dirs: &[PathBuf]) -> Vec<String> {
    let mut seen = BTreeMap::<String, &Path>::new();
    let mut warnings = Vec::new();

    for dir in skill_dirs {
        if !dir.exists() {
            continue;
        }
        for name in scan_names(dir) {
            if let Some(first_dir) = seen.get(&name) {
                warnings.push(format!(
                    "skill '{name}' from {} conflicts with skill from {}, skipping",
                    dir.display(),
                    first_dir.display(),
                ));
            } else {
                seen.insert(name, dir);
            }
        }
    }

    warnings
}

/// Scan a directory recursively for `SKILL.md` files and extract skill names.
pub fn scan_names(dir: &Path) -> Vec<String> {
    let mut results = Vec::new();
    scan_names_inner(dir, &mut results);
    results
}

fn scan_names_inner(dir: &Path, results: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with('.'))
        {
            continue;
        }

        let skill_file = path.join("SKILL.md");
        if skill_file.exists()
            && let Some(name) = extract_name(&skill_file)
        {
            results.push(name);
        }
        scan_names_inner(&path, results);
    }
}

/// Extract the `name` field from a SKILL.md YAML frontmatter.
///
/// Reads the one field rather than parsing the whole file — a scan touches
/// every skill on disk and never looks at a body.
fn extract_name(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let (frontmatter, _) = crate::md::split(&content).ok()?;
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("name:") {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}
