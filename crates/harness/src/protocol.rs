//! `protocol` — the runtime, as a harness sees it.
//!
//! One capability carries the whole of `ClientMessage`, because the message
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
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, OnceLock},
};
use wcore::protocol::message::{ClientMessage, ServerMessage, client_message, server_message};

/// How a harness reaches the runtime.
///
/// `Server::dispatch` is already the one door, but the trait is not
/// object-safe, so the daemon hands over a closure rather than itself — which
/// also keeps this crate from depending on the one that implements it.
pub type Dispatch = Arc<
    dyn Fn(ClientMessage) -> Pin<Box<dyn Future<Output = Vec<ServerMessage>> + Send>> + Send + Sync,
>;

/// Whether a message type is in a group this harness holds.
///
/// Default-deny: a message named in no group reaches nothing, and a group a
/// harness was not granted is the same. Anything destructive, anything that
/// answers on someone else's behalf, and anything whose payload is
/// substantially a credential belongs to no group a third party can hold.
pub fn allowed(message: &client_message::Msg, read: bool) -> bool {
    use client_message::Msg;
    match message {
        // protocol:read — the catalogue, and nothing that spends tokens.
        Msg::Ping(_)
        | Msg::GetStats(_)
        | Msg::ListAgents(_)
        | Msg::GetAgent(_)
        | Msg::ListSkills(_)
        | Msg::ListModels(_)
        | Msg::ListSubscriptions(_) => read,
        _ => false,
    }
}

/// Strip what a harness must not see from a reply.
///
/// `name` and `description` are what a caller actually reads off an agent —
/// `apps/tui/src/repl/delegate.rs` proves it, using exactly those two and
/// never touching `.config` — so blanking one field costs nothing real.
pub fn redact(mut reply: ServerMessage) -> ServerMessage {
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
pub fn call(protocol: &OnceLock<Dispatch>, request: &[u8], read: bool) -> Result<Vec<u8>> {
    let message = ClientMessage::decode(request)?;
    let Some(inner) = message.msg.as_ref() else {
        bail!("empty client message");
    };
    if !allowed(inner, read) {
        bail!("this message type is in no group this harness was granted");
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
    redact(reply).encode(&mut encoded)?;
    Ok(encoded)
}
