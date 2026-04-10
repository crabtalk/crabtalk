//! Crabtalk node — runtime + transport + event loop.

pub mod config;
pub mod cron;
pub mod event_bus;
pub mod hook;
pub mod mcp;
pub mod node;
pub mod provider;
pub mod repos;

pub use config::NodeConfig;
pub use hook::NodeEnv;
#[cfg(unix)]
pub use node::setup_socket;
pub use node::{
    Node, NodeHandle, bridge_shutdown,
    builder::{BuildProvider, DefaultProvider, build_default_provider},
    event::{NodeEvent, NodeEventSender},
    setup_tcp,
};
