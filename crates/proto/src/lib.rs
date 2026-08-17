//! The Crabtalk wire protocol.
//!
//! One schema, one crate, and the route between a message and a typed call.
//! Bare, the crate is `no_std` over an allocator — the generated messages and
//! nothing else, which is what a harness links (RFC 0205). Each feature adds
//! one half of the host's world: `server` dispatches an incoming message to a
//! handler, `client` builds one and unwraps the reply, and `llm` converts the
//! payloads the messages carry.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod client;
mod convert;
mod llm;
pub mod server;

/// The API traits the daemon and harnesses implement.
pub mod api {
    #[cfg(feature = "client")]
    pub use super::client::Client;
    #[cfg(feature = "server")]
    pub use super::server::Server;
}

include!(concat!(env!("OUT_DIR"), "/crabtalk.protocol.rs"));
