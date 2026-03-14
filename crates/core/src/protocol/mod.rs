//! Walrus wire protocol — message types, API traits, and wire codec.

pub mod api;
pub mod codec;
pub mod message;
pub mod proto;
pub mod whs;

/// Current protocol version.
pub const PROTOCOL_VERSION: &str = "0.2";
