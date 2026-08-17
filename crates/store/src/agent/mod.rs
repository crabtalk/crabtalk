//! What an agent *is*, as persisted. The loop that runs one lives in
//! `crabtalk-runtime`.

pub use config::{AgentConfig, DEFAULT_AGENT};
pub use id::AgentId;

mod config;
mod id;
