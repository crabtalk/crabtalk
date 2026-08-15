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
    // Hands. `bash`, `read`, and `edit` are a harness now rather than
    // something a client offers, so an agent that wants them has to say so —
    // and the default agent wants them.
    //
    // The root is the home directory, not the process's working directory: the
    // daemon usually starts from a service manager, where its cwd is wherever
    // launchd happened to put it — `~/.crabtalk` in practice, which would hand
    // the agent `exec` over the daemon's own tokens and memory. Home is the
    // widest thing that is still meaningfully a workspace, and it is narrower
    // than what the client-side tools it replaces could reach.
    cfg.harnesses = vec![wcore::HarnessConfig {
        name: "os".to_owned(),
        capabilities: vec!["fs".to_owned(), "exec".to_owned()],
        root: dirs::home_dir(),
    }];
    cfg
}
