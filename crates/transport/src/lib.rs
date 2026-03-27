//! Transport layer for the Crabtalk daemon.
//!
//! Wire message types, API traits, and codec live in `crabtalk-core::protocol`.
//! This crate provides UDS and TCP transport layers.

pub mod tcp;
#[cfg(unix)]
pub mod uds;
