//! Client tool declarations sent to the daemon on every stream.
//!
//! The daemon advertises whatever we declare here and forwards the calls
//! back — so this list is the client's capability surface, and since OS tools
//! became a harness that surface has shrunk to what genuinely needs a human or
//! a user interface.
//!
//! `bash`, `read`, and `edit` are deliberately absent. They run in the runtime
//! now, which is the machine that owns the files, and a client that declared
//! them would be claiming to execute what the daemon already did.

use crate::repl::delegate::Delegate;
use wcore::{agent::AsTool, protocol::message::ToolDef};

/// Tools for the user's own turn: the ask-user modal, and delegate.
///
/// Both are here because they need *this process*: one needs the person
/// sitting in front of it, the other renders sub-agent progress into this
/// REPL. Delegate stays client-side until it becomes a harness of its own
/// (RFC 0203, RFC 0205).
pub fn client_tools() -> Vec<ToolDef> {
    vec![
        client::tools::ask_user::schema().into(),
        Delegate::as_tool().into(),
    ]
}

/// Tools for a delegated sub-agent: none from this client.
///
/// Not a restriction — the opposite. A sub-agent's hands come from its own
/// agent config now, so it gets whatever harnesses that agent declares
/// without the orchestrating client having to offer anything. What it still
/// does not get is `ask_user`, because the REPL's modal is a single slot two
/// concurrent sub-agents would corrupt, and `delegate`, which caps recursion
/// at one level.
pub fn sub_agent_tools() -> Vec<ToolDef> {
    Vec::new()
}
