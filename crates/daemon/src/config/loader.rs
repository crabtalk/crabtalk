//! Configuration loading and first-run scaffolding.
//!
//! Handles filesystem I/O: reads agent prompt directories, scaffolds the
//! config directory structure on first run, and migrates from old layouts.

use anyhow::{Context, Result};
use std::path::Path;
use wcore::paths::{AGENTS_DIR, CONFIG_FILE, LOCAL_DIR, PACKAGES_DIR, SKILLS_DIR};

/// Default configuration template, embedded from the checked-in `config.toml`.
pub const DEFAULT_CONFIG: &str = include_str!("../../config.toml");

/// Load all agent markdown files from a directory as plain text.
///
/// Returns `(filename_stem, content)` pairs. Non-`.md` files are silently
/// skipped. Entries are sorted by filename for deterministic ordering.
/// Returns an empty vec if the directory does not exist.
pub fn load_agents_dir(path: &Path) -> Result<Vec<(String, String)>> {
    if !path.exists() {
        tracing::warn!("agent directory does not exist: {}", path.display());
        return Ok(Vec::new());
    }

    let mut entries: Vec<_> = std::fs::read_dir(path)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut agents = Vec::with_capacity(entries.len());
    for entry in entries {
        let stem = entry
            .path()
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let content = std::fs::read_to_string(entry.path())?;
        agents.push((stem, content));
    }

    Ok(agents)
}

/// Scaffold the full config directory structure on first run.
///
/// Runs migration for old layouts, then creates any missing directories
/// and writes a default `config.toml`.
pub fn scaffold_config_dir(config_dir: &Path) -> Result<()> {
    migrate_layout(config_dir);

    std::fs::create_dir_all(config_dir.join(AGENTS_DIR))
        .context("failed to create agents directory")?;
    std::fs::create_dir_all(config_dir.join(SKILLS_DIR))
        .context("failed to create skills directory")?;
    std::fs::create_dir_all(config_dir.join(PACKAGES_DIR))
        .context("failed to create packages directory")?;

    let config_toml = config_dir.join(CONFIG_FILE);
    if !config_toml.exists() {
        std::fs::write(&config_toml, DEFAULT_CONFIG)
            .with_context(|| format!("failed to write {}", config_toml.display()))?;
    }

    Ok(())
}

/// Migrate from the old config layout to the new package-centric layout.
///
/// - Renames `crab.toml` → `config.toml`
/// - Moves `skills/` → `local/skills/`
/// - Moves `agents/` → `local/agents/`
///
/// Each step is a no-op if already migrated. Errors are logged, not fatal.
fn migrate_layout(config_dir: &Path) {
    let old_config = config_dir.join("crab.toml");
    let new_config = config_dir.join(CONFIG_FILE);
    if old_config.exists() && !new_config.exists() {
        if let Err(e) = std::fs::rename(&old_config, &new_config) {
            tracing::warn!("failed to rename crab.toml → config.toml: {e}");
        } else {
            tracing::info!("migrated crab.toml → config.toml");
        }
    }

    let local_dir = config_dir.join(LOCAL_DIR);
    let _ = std::fs::create_dir_all(&local_dir);

    // Move skills/ → local/skills/ (only if old flat dir exists and new doesn't).
    let old_skills = config_dir.join("skills");
    let new_skills = config_dir.join(SKILLS_DIR);
    if old_skills.exists() && old_skills.is_dir() && !new_skills.exists() {
        if let Err(e) = std::fs::rename(&old_skills, &new_skills) {
            tracing::warn!("failed to move skills/ → local/skills/: {e}");
        } else {
            tracing::info!("migrated skills/ → local/skills/");
        }
    }

    // Move agents/ → local/agents/ (only if old flat dir exists and new doesn't).
    let old_agents = config_dir.join("agents");
    let new_agents = config_dir.join(AGENTS_DIR);
    if old_agents.exists() && old_agents.is_dir() && !new_agents.exists() {
        if let Err(e) = std::fs::rename(&old_agents, &new_agents) {
            tracing::warn!("failed to move agents/ → local/agents/: {e}");
        } else {
            tracing::info!("migrated agents/ → local/agents/");
        }
    }
}
