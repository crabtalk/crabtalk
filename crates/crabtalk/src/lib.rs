//! Crabtalk node — runtime + transports + protocol adapter.

pub mod hooks;
pub mod mcp;
pub mod node;
mod protocol;
pub mod provider;
#[cfg(feature = "fs")]
pub mod storage;

#[cfg(unix)]
pub use node::setup_socket;
pub use node::{Node, NodeHandle, bridge_shutdown, setup_tcp};
pub use wcore::NodeConfig;
