//! Hook module — re-exports Env<NodeHost, NodeRepos> as NodeEnv.

pub mod host;

/// The daemon's environment type — Env with NodeHost for
/// server-specific dispatch and NodeRepos for persistence.
pub type NodeEnv = runtime::Env<host::NodeHost, crate::repos::NodeRepos>;
