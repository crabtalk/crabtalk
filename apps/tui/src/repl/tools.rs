//! Client tool declarations sent to the daemon on every stream.
//!
//! The daemon advertises whatever we declare here and forwards the calls
//! back — so this list is the client's capability surface, and it differs
//! between the user's own turn and a delegated sub-agent's.

use crate::repl::delegate::Delegate;
use wcore::{agent::AsTool, protocol::message::ToolDef};

/// Tools for the user's own turn: OS tools, the ask-user modal, and
/// delegate.
pub fn client_tools() -> Vec<ToolDef> {
    let mut tools = hooks::os::schemas();
    tools.push(sdk::tools::ask_user::schema());
    tools.push(Delegate::as_tool());
    tools.into_iter().map(Into::into).collect()
}

/// Tools for a delegated sub-agent: hands, but no user and no further
/// fan-out. Both omissions are properties of *this client*, not of any
/// agent — the ask-user modal is a single slot, so two sub-agents asking at
/// once would corrupt it, and withholding delegate caps recursion at one
/// level without a depth counter.
///
/// What's left is an offer, not a grant: per-agent capability is
/// `AgentConfig.tools`, which narrows this list at `extend_tools` and is
/// enforced at dispatch. Scope a sub-agent by configuring the agent, not by
/// editing this list.
pub fn sub_agent_tools() -> Vec<ToolDef> {
    hooks::os::schemas().into_iter().map(Into::into).collect()
}
