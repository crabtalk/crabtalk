//! Crabtalk — runtime, hooks, and protocol.

pub mod hooks;
mod protocol;
pub mod provider;
pub mod storage;
pub mod system;

#[cfg(unix)]
pub use system::setup_socket;
pub use system::{CrabTalk, CrabTalkHandle, bridge_shutdown, setup_tcp};
pub use wcore::Config;
