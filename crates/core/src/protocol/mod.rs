//! Crabtalk wire protocol — message types, API traits, and wire codec.

pub use ::proto;

pub mod api;
pub mod codec;
pub mod message;

/// Current protocol version.
pub const PROTOCOL_VERSION: &str = "0.3";
