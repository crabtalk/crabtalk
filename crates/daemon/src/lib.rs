//! Crabtalk daemon — message central composing runtime, channels, and cron
//! scheduling. Personal agent, local-first.

pub mod config;
pub mod cron;
pub mod daemon;
pub mod event_bus;
pub mod hook;
pub mod provider;

pub use config::DaemonConfig;
#[cfg(unix)]
pub use daemon::setup_socket;
pub use daemon::{
    Daemon, DaemonHandle,
    builder::{BuildProvider, DefaultProvider, build_default_provider},
    bridge_shutdown,
    event::{DaemonEvent, DaemonEventSender},
    setup_tcp,
};
pub use hook::DaemonEnv;
