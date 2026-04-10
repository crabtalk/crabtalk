//! Hook module — re-exports Env<DaemonHost, DaemonRepos> as DaemonEnv.

pub mod host;

/// The daemon's environment type — Env with DaemonHost for
/// server-specific dispatch and DaemonRepos for persistence.
pub type DaemonEnv = runtime::Env<host::DaemonHost, crate::repos::DaemonRepos>;
