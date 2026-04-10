//! Crabtalk node — runtime + transport + event loop.

pub mod cron;
pub mod event_bus;
pub mod hook;
pub mod mcp;
pub mod node;
pub mod provider;
#[cfg(feature = "fs")]
pub mod storage;

pub use hook::NodeEnv;
#[cfg(unix)]
pub use node::setup_socket;
pub use node::{
    Node, NodeHandle, bridge_shutdown,
    builder::{BuildProvider, DefaultProvider, build_default_provider},
    event::{NodeEvent, NodeEventSender},
    setup_tcp,
};
pub use wcore::NodeConfig;
