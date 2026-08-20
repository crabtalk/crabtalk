//! Reach the Crabtalk runtime from a harness.
//!
//! [`berm-lang`](https://crates.io/crates/berm-lang) is the sandbox: it knows
//! how to be a harness, and its system set is the machine — files and commands,
//! the things a no_std RV64 guest cannot do for itself. It does not know what a
//! `ClientMessage` is, and must not, because the same sandbox runs elsewhere.
//!
//! This crate is the other half of that split, from the guest's side: the
//! stubs a harness calls to reach the `crabtalk` namespace.
//!
//! ```ignore
//! use berm_crabtalk::{protocol, proto::{ClientMessage, ListAgentsMsg, client_message}};
//!
//! let reply = protocol::call(ClientMessage {
//!     msg: Some(client_message::Msg::ListAgents(ListAgentsMsg {})),
//! })?;
//! ```

#![no_std]

extern crate alloc;

pub mod protocol;
pub mod sys;

pub use proto;
pub use sys::http;
