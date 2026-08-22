//! Crabtalk's side of berm.
//!
//! berm knows how to run a harness and nothing else. It serves no system
//! harness of its own — not the machine, and certainly not an agent or a
//! `ClientMessage` — because every one of those is a decision about a host, and
//! the same sandbox runs elsewhere. This crate is every such decision Crabtalk
//! makes:
//!
//! - [`BermHarness`], which surfaces a harness's tools to the runtime and
//!   dispatches calls to them
//! - [`fs`], [`exec`], [`Http`] and [`Protocol`], the implementations behind the
//!   `crabtalk` namespace — [`berm::Harness`] values, which is all an embedder
//!   ever hands over
//!
//! The split is what makes "berm is embeddable without crabtalk" a fact the
//! compiler checks rather than a promise: berm's dependency list has no
//! crabtalk crate in it, and cannot grow one without this file moving.

pub use harness::{BermHarness, bind};
pub use http::Http;
pub use protocol::{Dispatch, Protocol, Scope};

pub mod exec;
pub mod fs;

mod harness;
mod http;
mod protocol;
mod root;
mod sys;
