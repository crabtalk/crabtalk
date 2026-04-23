//! MCP (Model Context Protocol) client, bridge, and dispatcher.
//!
//! Three layers:
//! - [`client`] — minimal JSON-RPC 2.0 client over stdio or HTTP
//! - [`bridge`] — fleet of connected peers, tool cache, call routing
//! - [`handler`] / [`dispatch`] — config-driven load, port-file discovery,
//!   meta-tool dispatch
//!
//! # Features
//!
//! Pick exactly one HTTP backend and one TLS backend:
//! - `hyper` (default) / `reqwest`
//! - `native-tls` (default) / `rustls`
//!
//! The `hyper` backend is more compact — it skips reqwest's cookie store,
//! redirect logic, and decoders that MCP never uses.

#[cfg(all(feature = "reqwest", feature = "hyper"))]
compile_error!("features `reqwest` and `hyper` are mutually exclusive");
#[cfg(not(any(feature = "reqwest", feature = "hyper")))]
compile_error!("one of `reqwest` or `hyper` must be enabled");
#[cfg(all(feature = "native-tls", feature = "rustls"))]
compile_error!("features `native-tls` and `rustls` are mutually exclusive");
#[cfg(not(any(feature = "native-tls", feature = "rustls")))]
compile_error!("one of `native-tls` or `rustls` must be enabled");

pub use {bridge::McpBridge, handler::McpHandler};

pub mod bridge;
pub mod client;
pub mod dispatch;
pub mod handler;
