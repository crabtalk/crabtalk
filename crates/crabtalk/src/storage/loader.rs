//! Configuration loading and first-run scaffolding.
//!
//! Handles filesystem I/O: scaffolds the config directory structure on
//! first run and migrates from old layouts. Manifest resolution and agent
//! loading live in `wcore::config::manifest`.

use anyhow::{Context, Result};
use std::path::Path;
use wcore::paths::{AGENTS_DIR, CONFIG_FILE, LOCAL_DIR, PACKAGES_DIR, SETTINGS_FILE, SKILLS_DIR};

/// Default template for `config.toml` — the hand-edited install config
/// (LLM endpoint, task pool, env vars).
pub const DEFAULT_CONFIG: &str = include_str!("../../config.toml");

/// Default template for `local/settings.toml` — daemon-managed runtime
/// records (MCPs, agents). Overwritten on first daemon write.
pub const DEFAULT_SETTINGS: &str = include_str!("../../settings.toml");

/// Scaffold the full config directory structure on first run.
///
/// Runs migration for old layouts, then creates any missing directories
/// and writes default `config.toml` and `local/settings.toml`.
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

// ── Migration ───────────────────────────────────────────────────────

/// Migrate from old config layouts to the current package-based layout.
///
/// Phase 1: Renames `crab.toml` → `config.toml`, moves `skills/` and
/// `agents/` under `local/`.
///
/// Phase 2: Extracts `[mcps.*]` from `config.toml` into
/// `local/CrabTalk.toml`.
///
/// Phase 4: Flattens scope dirs in legacy `packages/`, renames `hub/`
/// → `registry/`.
///
/// Phase 5: Renames `plugins/` → `packages/` (the canonical name).
///
/// Each step is a no-op if already migrated. Errors are logged, not fatal.
fn migrate_layout(config_dir: &Path) {
    // Phase 1: rename crab.toml → config.toml
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

    // Phase 1: move skills/ → local/skills/
    let old_skills = config_dir.join("skills");
    let new_skills = config_dir.join(SKILLS_DIR);
    if old_skills.exists() && old_skills.is_dir() && !new_skills.exists() {
        if let Err(e) = std::fs::rename(&old_skills, &new_skills) {
            tracing::warn!("failed to move skills/ → local/skills/: {e}");
        } else {
            tracing::info!("migrated skills/ → local/skills/");
        }
    }

    // Phase 1: move agents/ → local/agents/
    let old_agents = config_dir.join("agents");
    let new_agents = config_dir.join(AGENTS_DIR);
    if old_agents.exists() && old_agents.is_dir() && !new_agents.exists() {
        if let Err(e) = std::fs::rename(&old_agents, &new_agents) {
            tracing::warn!("failed to move agents/ → local/agents/: {e}");
        } else {
            tracing::info!("migrated agents/ → local/agents/");
        }
    }

    // Phase 2: extract [mcps] from config.toml → local/CrabTalk.toml
    let config_path = config_dir.join(CONFIG_FILE);
    if config_path.exists() {
        migrate_mcps(&config_path, &local_dir.join("CrabTalk.toml"));
    }

    // Phase 3: move [disabled] from local/CrabTalk.toml → config.toml
    let manifest_path = local_dir.join("CrabTalk.toml");
    if manifest_path.exists() && config_path.exists() {
        migrate_disabled(&manifest_path, &config_path);
    }

    // Phase 4: flatten legacy scoped packages/scope/name.toml → packages/name.toml
    let packages_dir = config_dir.join(PACKAGES_DIR);
    if packages_dir.exists() && packages_dir.is_dir() {
        let scopes: Vec<_> = std::fs::read_dir(&packages_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .collect();
        for scope_entry in scopes {
            let scope_path = scope_entry.path();
            if let Ok(manifests) = std::fs::read_dir(&scope_path) {
                for manifest in manifests.flatten() {
                    let src = manifest.path();
                    if src.extension().is_some_and(|e| e == "toml") {
                        let dst = packages_dir.join(manifest.file_name());
                        let _ = std::fs::rename(&src, &dst);
                    }
                }
            }
            let _ = std::fs::remove_dir_all(&scope_path);
        }
    }

    // Phase 4: rename hub/ → registry/
    let old_hub = config_dir.join("hub");
    let new_registry = config_dir.join("registry");
    if old_hub.exists() && old_hub.is_dir() && !new_registry.exists() {
        if let Err(e) = std::fs::rename(&old_hub, &new_registry) {
            tracing::warn!("failed to rename hub/ → registry/: {e}");
        } else {
            tracing::info!("migrated hub/ → registry/");
        }
    }

    // Phase 5: rename plugins/ → packages/ (canonical name).
    let old_plugins = config_dir.join("plugins");
    let new_packages = config_dir.join(PACKAGES_DIR);
    if old_plugins.exists() && old_plugins.is_dir() && !new_packages.exists() {
        if let Err(e) = std::fs::rename(&old_plugins, &new_packages) {
            tracing::warn!("failed to rename plugins/ → packages/: {e}");
        } else {
            tracing::info!("migrated plugins/ → packages/");
        }
    }
}

/// Extract `[mcps.*]` from config.toml into `local/CrabTalk.toml`,
/// removing it from config.toml.
fn migrate_mcps(config_path: &Path, manifest_path: &Path) {
    use toml_edit::DocumentMut;

    let Ok(content) = std::fs::read_to_string(config_path) else {
        return;
    };
    let Ok(mut doc) = content.parse::<DocumentMut>() else {
        return;
    };

    let has_mcps = doc
        .get("mcps")
        .and_then(|v| v.as_table())
        .is_some_and(|t| !t.is_empty());
    if !has_mcps {
        return;
    }

    // Build or load the manifest document.
    let mut manifest_doc = if manifest_path.exists() {
        std::fs::read_to_string(manifest_path)
            .ok()
            .and_then(|s| s.parse::<DocumentMut>().ok())
            .unwrap_or_default()
    } else {
        DocumentMut::default()
    };

    // Only migrate if manifest doesn't already have [mcps].
    if manifest_doc
        .get("mcps")
        .and_then(|v| v.as_table())
        .is_none_or(|t| t.is_empty())
        && let Some(mcps) = doc.remove("mcps")
    {
        manifest_doc.insert("mcps", mcps);
        tracing::info!("migrated [mcps] from config.toml → local/CrabTalk.toml");
    }

    // Write both files back.
    if let Err(e) = std::fs::write(manifest_path, manifest_doc.to_string()) {
        tracing::warn!("failed to write local/CrabTalk.toml: {e}");
        return;
    }
    if let Err(e) = std::fs::write(config_path, doc.to_string()) {
        tracing::warn!("failed to update config.toml after migration: {e}");
    }
}

/// Move `[disabled]` from `local/CrabTalk.toml` to `config.toml`.
fn migrate_disabled(manifest_path: &Path, config_path: &Path) {
    use toml_edit::DocumentMut;

    let Ok(manifest_content) = std::fs::read_to_string(manifest_path) else {
        return;
    };
    let Ok(mut manifest_doc) = manifest_content.parse::<DocumentMut>() else {
        return;
    };

    let has_disabled = manifest_doc
        .get("disabled")
        .and_then(|v| v.as_table())
        .is_some_and(|t| !t.is_empty());
    if !has_disabled {
        return;
    }

    let Ok(config_content) = std::fs::read_to_string(config_path) else {
        return;
    };
    let Ok(mut config_doc) = config_content.parse::<DocumentMut>() else {
        return;
    };

    // Only migrate if config.toml doesn't already have [disabled].
    if config_doc
        .get("disabled")
        .and_then(|v| v.as_table())
        .is_some_and(|t| !t.is_empty())
    {
        return;
    }

    if let Some(disabled) = manifest_doc.remove("disabled") {
        config_doc.insert("disabled", disabled);
        if let Err(e) = std::fs::write(config_path, config_doc.to_string()) {
            tracing::warn!("failed to write config.toml during disabled migration: {e}");
            return;
        }
        if let Err(e) = std::fs::write(manifest_path, manifest_doc.to_string()) {
            tracing::warn!("failed to update local/CrabTalk.toml after disabled migration: {e}");
            return;
        }
        tracing::info!("migrated [disabled] from local/CrabTalk.toml → config.toml");
    }
}
