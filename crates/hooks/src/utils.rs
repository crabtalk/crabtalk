//! Helpers that belong to no single hook.

use wcore::AgentConfig;

/// Construct the default `crab` agent with the given model.
///
/// Lives here rather than in a storage backend because every backend
/// seeds it and none owns it: `scaffold` calls this to populate a fresh
/// store, and the daemon falls back to it when no `crab` is persisted.
/// Callers must supply a model — an agent without one can't run.
pub fn default_crab(model: impl Into<String>) -> AgentConfig {
    let mut cfg = AgentConfig::new(wcore::paths::DEFAULT_AGENT);
    cfg.system_prompt = crate::DEFAULT_SOUL.to_owned();
    cfg.model = model.into();
    cfg
}
