//! The Crabtalk wire protocol.
//!
//! One schema, one crate, two worlds: `std` carries serde derives and the
//! fallible conversions the daemon needs, and without it the same messages
//! build `no_std` over an allocator so a harness speaks the daemon's own
//! vocabulary rather than a second one invented for it (RFC 0205).

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

mod convert;
mod llm;

include!(concat!(env!("OUT_DIR"), "/crabtalk.protocol.rs"));
