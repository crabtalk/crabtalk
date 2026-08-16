//! Crabtalk's side of berm.
//!
//! berm knows how to run a harness and how to bound one to a directory. It does
//! not know what an agent is, what a `ClientMessage` is, or that a daemon
//! exists — and it must not, because the same sandbox runs elsewhere. This
//! crate is everything that knowledge lives in:
//!
//! - [`HarnessHook`], which surfaces a harness's tools to the runtime and
//!   dispatches calls to them
//! - the `crabtalk.protocol.call` capability, which is a [`berm::Capability`]
//!   like any other an embedder supplies
//! - `crabtalk.http.fetch`, which is here for a second reason as well: hyper
//!   needs a reactor, and the sandbox is sync and has none
//!
//! The split is what makes "berm is embeddable without crabtalk" a fact the
//! compiler checks rather than a promise: berm's dependency list has no
//! crabtalk crate in it, and cannot grow one without this file moving.

mod hook;
mod http;
mod protocol;

pub use hook::HarnessHook;
pub use http::call as http_fetch;
pub use protocol::{Dispatch, Scope, call as protocol_call};
