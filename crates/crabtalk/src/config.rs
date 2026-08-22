//! What the daemon is started with.
//!
//! One structure, assembled by the embedder. Nothing here is read from
//! disk by this crate: the settings arrive already parsed, and the two
//! directories are locations only a process that knows its own install
//! can name. There are no defaults for the same reason.
//!
//! Directories, not documents. What is written by hand is parsed before
//! it gets here, and what the daemon writes is store state — so the
//! paths that remain are the ones holding artifacts it reads on demand.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    /// The hand-written settings, parsed by whoever starts the daemon.
    pub settings: store::Config,
    /// Harness images, one `{name}.elf` each, loaded as agents declare them.
    pub harnesses: PathBuf,
    /// Where the harness engine caches generated code.
    pub cache: PathBuf,
}
