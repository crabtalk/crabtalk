//! Hook module — re-exports Env<NodeHost, FsStorage> as NodeEnv.

pub mod host;

/// The daemon's environment type — Env with NodeHost for
/// server-specific dispatch and FsStorage for persistence.
pub type NodeEnv = runtime::Env<host::NodeHost, crate::storage::FsStorage>;
