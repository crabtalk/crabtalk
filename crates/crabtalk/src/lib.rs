//! Crabtalk daemon — runtime + transports + protocol adapter.

pub mod daemon;
pub mod hooks;
pub mod mcp;
mod protocol;
pub mod provider;
pub mod storage;

#[cfg(unix)]
pub use daemon::setup_socket;
pub use daemon::{Daemon, DaemonHandle, bridge_shutdown, setup_tcp};
pub use wcore::DaemonConfig;
