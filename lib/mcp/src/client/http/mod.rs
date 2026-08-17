//! HTTP transport for JSON-RPC — exactly one backend is compiled in via
//! the mutually-exclusive `hyper` / `reqwest` crate features.

#[cfg(feature = "hyper")]
mod hyper;
#[cfg(feature = "reqwest")]
mod reqwest;

#[cfg(feature = "hyper")]
pub use hyper::HttpTransport;
#[cfg(feature = "reqwest")]
pub use reqwest::HttpTransport;
