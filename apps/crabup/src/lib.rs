//! crabup — version manager for the Crabtalk ecosystem.
//!
//! Bare, this crate is the install layout and nothing else: crabup creates
//! `~/.crabtalk`, the way rustup creates `~/.rustup`, so it is where the
//! layout is defined and everything else reads it from here rather than
//! re-deriving it. The commands that act on it are the binary's half.

pub use dirs::{BIN_DIR, CONFIG_DIR};

pub mod dirs;

#[cfg(feature = "cmd")]
pub mod cmd;
