//! Protocol types, for a guest.
//!
//! Generated from the same `crabtalk.proto` the host compiles, so a harness
//! speaks the daemon's own vocabulary rather than a second one invented for
//! it (RFC 0205). The two emissions differ only in the world they target:
//! the host's is `std` and carries serde derives, this one is `no_std` over
//! an allocator.

#![no_std]

extern crate alloc;
// prost's generated code writes `::prost::…` paths, and the workspace has to
// alias the crate to build it without std — the host's copy of the same
// dependency keeps its default features.
extern crate prost_guest as prost;

include!(concat!(env!("OUT_DIR"), "/crabtalk.protocol.rs"));
