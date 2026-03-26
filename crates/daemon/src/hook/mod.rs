//! Hook module — re-exports RuntimeHook<DaemonBridge> as DaemonHook.

pub mod bridge;

/// The daemon's hook type — RuntimeHook with DaemonBridge for server-specific dispatch.
pub type DaemonHook = runtime::RuntimeHook<bridge::DaemonBridge>;
