//! Hook module — re-exports Env<DaemonBackend> as DaemonEnv.

pub mod backend;

/// The daemon's environment type — Env with DaemonBackend for server-specific dispatch.
pub type DaemonEnv = runtime::Env<backend::DaemonBackend>;
