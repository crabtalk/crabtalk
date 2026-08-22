//! Crabtalk — runtime, hooks, and protocol.

pub mod harness;
mod protocol;
pub mod system;

pub use crabllm_core as llm;
pub use store::Config;
pub use system::{CrabTalk, CrabTalkHandle};
