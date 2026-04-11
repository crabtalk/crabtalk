//! Built-in tool implementations for Crabtalk.
//!
//! - [`os`]: bash, read, edit handlers
//! - [`ask_user`]: structured question handler
//! - [`memory`]: Memory struct + recall/remember/forget handlers
//! - [`skill`]: skill loader + tool handler

pub mod ask_user;
pub mod memory;
pub mod os;
pub mod skill;

pub use memory::Memory;
