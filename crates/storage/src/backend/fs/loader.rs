//! Configuration loading and first-run scaffolding.
//!
//! Handles filesystem I/O: scaffolds the config directory structure on
//! first run. Manifest resolution and agent loading live in
//! `wcore::config::manifest`.
#![allow(dead_code)]

use anyhow::{Context, Result};
use std::path::Path;
use wcore::paths::{AGENTS_DIR, CONFIG_FILE, PACKAGES_DIR, SETTINGS_FILE, SKILLS_DIR};

/// Default template for `config.toml` — the hand-edited install config
/// (LLM endpoint, task pool, env vars).
pub const DEFAULT_CONFIG: &str = include_str!("../../../config.toml");

/// Default template for `local/settings.toml` — daemon-managed runtime
/// records (MCPs, agents). Overwritten on first daemon write.
pub const DEFAULT_SETTINGS: &str = include_str!("../../../settings.toml");

/// Scaffold the full config directory structure on first run.
///
/// Creates any missing directories and writes default `config.toml` and
/// `local/settings.toml`.
pub fn scaffold_config_dir(config_dir: &Path) -> Result<()> {
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

    let settings_toml = config_dir.join(SETTINGS_FILE);
    if !settings_toml.exists() {
        if let Some(parent) = settings_toml.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        std::fs::write(&settings_toml, DEFAULT_SETTINGS)
            .with_context(|| format!("failed to write {}", settings_toml.display()))?;
    }

    Ok(())
}
