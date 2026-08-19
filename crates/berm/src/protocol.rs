//! `protocol` — the runtime, as a harness sees it.
//!
//! One system harness carries the whole of `ClientMessage`, because the message
//! type is already a discriminant inside the payload and spending a second one
//! on the ABI would duplicate what protobuf carries (RFC 0205). The grant is
//! therefore two-level: the number gates the family, and this allowlist gates
//! which message types pass, checked once on decode.
//!
//! One door also means one place to redact. `AgentInfo.config` is the full
//! `AgentConfig` as JSON, and an agent's config holds its MCPs by value —
//! `env` and a literal `Authorization` header among them. Handing that to a
//! harness would make every protocol read a credential read, so the field is
//! blanked here. That is this boundary paying for a bill RFC 0193 deferred,
//! and it holds the line rather than settling it.

use anyhow::{Result, bail};
use prost::Message;
use proto::{ClientMessage, ServerMessage, client_message, server_message};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, OnceLock},
};
use store::AgentId;

/// What the harness calls to reach the runtime. Named `crabtalk.` rather than
/// `berm.` because only crabtalk implements it — a different embedder has its
/// own API and would name a system harness after that.
pub(crate) const CALL: &str = "crabtalk.protocol.call";

/// How a harness reaches the runtime.
///
/// `Server::dispatch` is already the one door, but the trait is not
/// object-safe, so the daemon hands over a closure rather than itself — which
/// also keeps this crate from depending on the one that implements it.
pub type Dispatch = Arc<
    dyn Fn(ClientMessage) -> Pin<Box<dyn Future<Output = Vec<ServerMessage>> + Send>> + Send + Sync,
>;

/// What one agent's declaration granted the harness it loaded.
///
/// The grant lives in the declaration, so the agent's own limits are known
/// here without the invocation having to carry them. That is also why this is
/// part of an image's digest: two agents declaring the same ELF under
/// different scopes are two sandboxes, not one.
pub struct Scope {
    /// Whether `protocol:read` was granted.
    pub read: bool,
    /// Whether `protocol:sessions` was granted.
    pub sessions: bool,
    /// The skills this agent declared. Empty is unrestricted, which is what
    /// an agent naming none has always meant.
    pub skills: Vec<String>,
    /// The agent that declared the harness. Session search is narrowed to it
    /// rather than filtered by it: a harness asking for someone else's
    /// conversations is answered about its own.
    pub agent: AgentId,
}

impl Scope {
    /// Whether `name` is a skill this agent may reach.
    fn may_use(&self, name: &str) -> bool {
        self.skills.is_empty() || self.skills.iter().any(|s| s == name)
    }

    /// Drop what the agent did not declare from a catalogue listing.
    fn narrow(&self, mut reply: ServerMessage) -> ServerMessage {
        if let Some(server_message::Msg::SkillList(list)) = reply.msg.as_mut() {
            list.skills.retain(|skill| self.may_use(&skill.name));
        }
        reply
    }
}

impl Scope {
    /// Whether a message type is in a group this harness holds.
    ///
    /// Default-deny: a message named in no group reaches nothing, and a group
    /// a harness was not granted is the same. Anything destructive, anything
    /// that answers on someone else's behalf, and anything whose payload is
    /// substantially a credential belongs to no group a third party can hold.
    fn allows(&self, message: &client_message::Msg) -> bool {
        use client_message::Msg;
        match message {
            // protocol:read — the catalogue, and nothing that spends tokens.
            Msg::Ping(_)
            | Msg::GetStats(_)
            | Msg::ListAgents(_)
            | Msg::GetAgent(_)
            | Msg::ListSkills(_)
            | Msg::GetSkill(_)
            | Msg::ListModels(_)
            | Msg::ListSubscriptions(_) => self.read,
            // protocol:sessions — excerpts of the declaring agent's own past
            // conversations. Its own group rather than part of `read`, which
            // is the catalogue: this is content, and content is not a listing.
            Msg::SearchSessions(_) => self.sessions,
            _ => false,
        }
    }
}

/// Strip what a harness must not see from a reply.
///
/// `name` and `description` are what a caller actually reads off an agent —
/// the `peers` harness uses exactly those two and never touches `.config` —
/// so blanking one field costs nothing real.
pub(crate) fn redact(mut reply: ServerMessage) -> ServerMessage {
    match reply.msg.as_mut() {
        Some(server_message::Msg::AgentInfo(info)) => info.config.clear(),
        Some(server_message::Msg::AgentList(list)) => {
            for info in &mut list.agents {
                info.config.clear();
            }
        }
        _ => {}
    }
    reply
}

/// Decode one `ClientMessage`, check it against the allowlist, dispatch it,
/// and encode the reply.
///
/// The dispatcher arrives after the harnesses do — the daemon that implements
/// it is built on top of them — so it is read through a `OnceLock` rather than
/// held. A call before it is connected fails rather than waiting.
pub fn call(protocol: &OnceLock<Dispatch>, request: &[u8], scope: &Scope) -> Result<Vec<u8>> {
    let mut message = ClientMessage::decode(request)?;
    let Some(inner) = message.msg.as_ref() else {
        bail!("empty client message");
    };
    if !scope.allows(inner) {
        bail!("this message type is in no group this harness was granted");
    }
    // Refused here rather than filtered out of the reply, so asking for a
    // skill outside the declaration costs nothing and says so.
    if let client_message::Msg::GetSkill(msg) = inner
        && !scope.may_use(&msg.name)
    {
        bail!("skill not available: {}", msg.name);
    }

    // Overwritten rather than checked: the agent filter is not the harness's to
    // choose, and refusing a wrong one would only teach it to send the right
    // one. `sender` stays free — an agent's own conversations span every
    // partner it has, and it can already resume any of them.
    if let Some(client_message::Msg::SearchSessions(msg)) = message.msg.as_mut() {
        msg.agent = scope.agent.to_string();
    }

    let Some(dispatch) = protocol.get() else {
        bail!("the protocol is not connected yet");
    };
    let handle = tokio::runtime::Handle::try_current()
        .map_err(|_| anyhow::anyhow!("the protocol needs a running reactor"))?;
    let replies = handle.block_on(dispatch(message));

    // Every message in a read group is request-response. Streaming ones are
    // in no group a harness can hold, so one reply is the whole answer.
    let Some(reply) = replies.into_iter().next() else {
        bail!("the runtime returned no reply");
    };

    let mut encoded = Vec::new();
    scope.narrow(redact(reply)).encode(&mut encoded)?;
    Ok(encoded)
}
