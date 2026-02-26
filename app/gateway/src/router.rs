//! Channel routing table.
//!
//! Maps incoming channel events to the correct agent based on platform
//! and channel ID with three-tier fallback: exact match, platform
//! catch-all, default agent (DD#3).

use agent::Platform;
use compact_str::CompactString;

/// A routing rule mapping platform/channel to an agent.
#[derive(Debug, Clone)]
pub struct RoutingRule {
    /// Target platform.
    pub platform: Platform,
    /// Optional channel ID for exact matching.
    pub channel_id: Option<CompactString>,
    /// Target agent name.
    pub agent: CompactString,
}

/// Routes channel events to agents via three-tier fallback.
#[derive(Debug)]
pub struct ChannelRouter {
    rules: Vec<RoutingRule>,
    default_agent: Option<CompactString>,
}

impl ChannelRouter {
    /// Create a new router with rules and an optional default agent.
    pub fn new(rules: Vec<RoutingRule>, default_agent: Option<CompactString>) -> Self {
        Self {
            rules,
            default_agent,
        }
    }

    /// Resolve the target agent for a given platform and channel ID.
    ///
    /// Fallback order (DD#3):
    /// 1. Exact match (platform + channel_id)
    /// 2. Platform catch-all (platform only, no channel_id in rule)
    /// 3. Default agent
    pub fn route(&self, platform: Platform, channel_id: &str) -> Option<&CompactString> {
        // 1. Exact match
        for rule in &self.rules {
            if rule.platform == platform
                && let Some(ref id) = rule.channel_id
                && id == channel_id
            {
                return Some(&rule.agent);
            }
        }

        // 2. Platform catch-all
        for rule in &self.rules {
            if rule.platform == platform && rule.channel_id.is_none() {
                return Some(&rule.agent);
            }
        }

        // 3. Default
        self.default_agent.as_ref()
    }

    /// Get the default agent.
    pub fn default_agent(&self) -> Option<&CompactString> {
        self.default_agent.as_ref()
    }
}

/// Parse a platform name string into a `Platform` enum.
pub fn parse_platform(name: &str) -> anyhow::Result<Platform> {
    match name.to_lowercase().as_str() {
        "telegram" => Ok(Platform::Telegram),
        other => anyhow::bail!("unknown platform: {other}"),
    }
}
