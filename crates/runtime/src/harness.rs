//! Harness — the seam subsystems implement to participate in the runtime
//! lifecycle: schemas, usage, per-agent lifecycle, and tool dispatch.
//! Composition lives in the assembly crate, not here.

use crate::{AgentEvent, ToolDispatch, ToolFuture};
use crabllm_core::Tool;
use store::{AgentConfig, AgentId};

/// A pluggable subsystem that participates in the agent lifecycle.
///
/// All methods have default no-op implementations so subsystems only
/// override what they need.
pub trait Harness: Send + Sync {
    /// Tool schemas this harness provides.
    fn schema(&self) -> Vec<Tool> {
        vec![]
    }

    /// When to reach for this harness's tools, and how they go together.
    ///
    /// A declaration rather than a lifecycle callback: it answers a question
    /// no single tool's `description` can, because it is about choosing
    /// between them. It is in context from the first turn — the model has to
    /// have it *before* it decides — so it is paid on every turn and belongs
    /// at that altitude. Anything longer than a few lines is a skill.
    fn usage(&self) -> Option<String> {
        None
    }

    /// Called before an `Agent` is built for a run.
    fn on_build_agent(&self, config: AgentConfig) -> AgentConfig {
        config
    }

    /// Called each time an agent is resolved for a run, before the
    /// `Agent` is built. Harnesses that track per-agent state (e.g. scopes,
    /// descriptions, sandboxes) record it here, so it is in place before
    /// the run starts.
    ///
    /// **Must be idempotent.** There is no registry to fire this once
    /// per agent: it fires per run, for the agent that is running, and
    /// an implementation that re-did its work every time would pay that
    /// cost on every message.
    fn on_resolve_agent(&self, _id: &AgentId, _config: &AgentConfig) {}

    /// Called after an agent is deleted from storage. Harnesses drop any
    /// per-agent state they own — nothing will resolve this id again.
    fn on_forget_agent(&self, _id: &AgentId) {}

    /// Called by Runtime after each agent step during execution.
    fn on_event(&self, _agent: &AgentId, _session_id: u64, _event: &AgentEvent) {}

    /// Preprocess user content before it becomes a message.
    /// Return `Some(modified)` to transform, `None` to pass through.
    fn preprocess(&self, _agent: &AgentId, _content: &str) -> Option<String> {
        None
    }

    /// Tools to include when building a scoped agent's whitelist, plus an
    /// optional scope prompt line (e.g. `"skills: foo, bar"`).
    ///
    /// Default: include all tools from `schema()` unconditionally, no
    /// scope line. Override to gate inclusion on agent config fields.
    fn scoped_tools(&self, _config: &AgentConfig) -> (Vec<String>, Option<String>) {
        let tools = self
            .schema()
            .iter()
            .map(|t| t.function.name.clone())
            .collect();
        (tools, None)
    }

    /// Dispatch a tool call by name. Return `None` if this harness
    /// doesn't own the tool.
    fn dispatch<'a>(&'a self, _name: &'a str, _call: ToolDispatch) -> Option<ToolFuture<'a>> {
        None
    }
}

/// No-op Harness for tests.
impl Harness for () {}
