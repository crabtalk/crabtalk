//! Crabtalk — runtime, hooks, and protocol.

pub mod bridge;
pub mod hooks;
mod protocol;
pub mod provider;
pub mod storage;
pub mod system;

pub use crabllm_core as llm;
pub use crabllm_provider as llmp;
#[cfg(unix)]
pub use system::setup_socket;
pub use system::{bridge_shutdown, setup_tcp, CrabTalk, CrabTalkHandle};
pub use wcore::Config;
