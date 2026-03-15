//! Configuration loading and first-run scaffolding.
//!
//! Handles filesystem I/O: reads agent prompt directories and scaffolds the
//! config directory structure on first run.

use crate::config::DaemonConfig;
use anyhow::{Context, Result};
use std::path::Path;
use wcore::paths::{AGENTS_DIR, DATA_DIR, SKILLS_DIR};

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
/// Creates subdirectories (agents, skills, data) and writes a default walrus.toml.
pub fn scaffold_config_dir(config_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(config_dir.join(AGENTS_DIR))
        .context("failed to create agents directory")?;
    std::fs::create_dir_all(config_dir.join(SKILLS_DIR))
        .context("failed to create skills directory")?;
    std::fs::create_dir_all(config_dir.join(DATA_DIR))
        .context("failed to create data directory")?;

    let config_toml = config_dir.join("walrus.toml");
    if !config_toml.exists() {
        let contents = toml::to_string_pretty(&DaemonConfig::default())
            .context("failed to serialize default config")?;
        std::fs::write(&config_toml, contents)
            .with_context(|| format!("failed to write {}", config_toml.display()))?;
    }

    Ok(())
}
