//! Crabtalk node — runtime + transports + protocol adapter.

pub(crate) mod builder;
pub mod cron;
pub mod delegate;
pub mod event;
pub mod hook;
pub mod mcp;
pub mod node;
mod protocol;
pub mod provider;
#[cfg(feature = "fs")]
pub mod storage;

pub use builder::{BuildProvider, DefaultProvider, build_default_provider};
pub use hook::NodeEnv;
#[cfg(unix)]
pub use node::setup_socket;
pub use node::{Node, NodeHandle, bridge_shutdown, setup_tcp};
pub use tools::Memory;
pub use wcore::NodeConfig;
