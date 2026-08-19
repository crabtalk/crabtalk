//! Crabtalk's side of berm.
//!
//! berm knows how to run a harness and how to bound one to a directory. It does
//! not know what an agent is, what a `ClientMessage` is, or that a daemon
//! exists — and it must not, because the same sandbox runs elsewhere. This
//! crate is everything that knowledge lives in:
//!
//! - [`BermHarness`], which surfaces a harness's tools to the runtime and
//!   dispatches calls to them
//! - the `crabtalk.protocol.call` system harness, which is a [`berm::Harness`]
//!   like any other an embedder supplies
//! - `crabtalk.http.fetch`, which is here for a second reason as well: hyper
//!   needs a reactor, and the sandbox is sync and has none
//!
//! The split is what makes "berm is embeddable without crabtalk" a fact the
//! compiler checks rather than a promise: berm's dependency list has no
//! crabtalk crate in it, and cannot grow one without this file moving.

use std::{path::PathBuf, sync::LazyLock};

mod harness;
mod http;
mod protocol;

pub use harness::BermHarness;
pub use http::call as http_fetch;
pub use protocol::{Dispatch, Scope, call as protocol_call};

/// Harness images (`~/.crabtalk/harnesses/`), one `{name}.elf` each.
///
/// Lives here rather than with the rest of the install layout because this
/// crate is the only thing that loads an image — and the installer that will
/// write them reaches crabtalk's side of berm, not the other way round.
pub static HARNESSES_DIR: LazyLock<PathBuf> =
    LazyLock::new(|| crabup::CONFIG_DIR.join("harnesses"));
