//! Crabtalk — runtime, hooks, and protocol.

mod config;
pub mod harness;
mod protocol;
pub mod system;

pub use config::Config;
pub use crabllm_core as llm;
pub use system::{CrabTalk, CrabTalkHandle};
