//! First-startup scaffold: create the directory layout and seed the
//! built-in `crab` agent. Owns the default crab definition because it
//! is the only producer of fresh installs.

use super::FsStorage;
use anyhow::Result;
use std::fs;
use wcore::AgentConfig;

/// Built-in crab agent prompt (from `prompts/crab.md`).
const CRAB_PROMPT: &str = crate::hooks::memory::DEFAULT_SOUL;

/// Construct the default `crab` system agent with the given model.
///
/// Used by [`scaffold`] to seed a fresh install and by the daemon as a
/// fallback when no `crab` agent is stored. Callers must provide a model
/// — agents without a model can't run.
pub fn default_crab(model: impl Into<String>) -> AgentConfig {
    let mut cfg = AgentConfig::new(wcore::paths::DEFAULT_AGENT);
    cfg.system_prompt = CRAB_PROMPT.to_owned();
    cfg.model = model.into();
    cfg
}

pub(super) fn scaffold(storage: &FsStorage, default_model: &str) -> Result<()> {
    fs::create_dir_all(&storage.config_dir)?;
    fs::create_dir_all(storage.config_dir.join(wcore::paths::LOCAL_DIR))?;
    fs::create_dir_all(storage.config_dir.join(wcore::paths::SKILLS_DIR))?;
    fs::create_dir_all(storage.config_dir.join(wcore::paths::AGENTS_DIR))?;
    fs::create_dir_all(&storage.sessions_root)?;

    let file = storage.read_settings()?;
    if file.agents.is_empty() {
        let crab = default_crab(default_model);
        let prompt = crab.system_prompt.clone();
        super::agents::upsert_agent(storage, &crab, &prompt)?;
    }
    Ok(())
}
