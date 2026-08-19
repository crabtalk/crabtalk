//! Cache budgets.

use serde::{Deserialize, Serialize};

/// What the daemon may hold in memory (`[cache]` in `config.toml`), one
/// slot per cache, in megabytes.
///
/// A slot belongs here when what it holds is a copy of something the
/// store already has, sized by traffic rather than by configuration.
/// Everything else the daemon keeps resident is bounded by what a person
/// declared — harness images by the agents that use them, MCP peers by
/// the servers configured — and is swept when a declaration changes
/// rather than priced here.
///
/// Lives in this crate because `config.toml` is parsed here; the caches
/// themselves are owned by whoever holds the data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    /// Session history held resident, or `None` for no bound.
    ///
    /// Megabytes rather than a session count because the two barely
    /// relate: a fresh session costs nothing and one filling 60% of a
    /// 1M-token context measures ~3 MB, so "ten sessions" is anywhere
    /// between nothing and thirty megabytes. The default is that ten,
    /// priced.
    ///
    /// Eviction drops a copy rather than state — a run persists before
    /// it returns, so an evicted session reloads on its next message —
    /// and a session mid-run is never evicted, which makes this a target
    /// rather than a ceiling.
    pub sessions: Option<usize>,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self { sessions: Some(32) }
    }
}
