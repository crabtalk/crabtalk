//! Crabtalk — runtime, hooks, and protocol.

pub mod bridge;
mod protocol;
pub mod system;

pub use crabllm_core as llm;
pub use store::Config;
#[cfg(unix)]
pub use system::setup_socket;
pub use system::{CrabTalk, CrabTalkHandle, bridge_shutdown, setup_tcp};
