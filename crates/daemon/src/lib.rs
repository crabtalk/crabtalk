//! Crabtalk daemon — message central composing runtime, channels, and cron
//! scheduling. Personal agent, local-first.

pub mod config;
pub mod cron;
pub mod daemon;
pub mod event_bus;
pub mod hook;
pub mod mcp;
pub mod provider;
pub mod repos;

pub use config::DaemonConfig;
#[cfg(unix)]
pub use daemon::setup_socket;
pub use daemon::{
    Daemon, DaemonHandle, bridge_shutdown,
    builder::{BuildProvider, DefaultProvider, build_default_provider},
    event::{DaemonEvent, DaemonEventSender},
    setup_tcp,
};
pub use hook::DaemonEnv;
