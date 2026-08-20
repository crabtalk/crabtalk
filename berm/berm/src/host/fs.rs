//! `fs` — files under a granted root.
//!
//! Read and write are separate system harnesses, so a harness that summarises a
//! directory can be given the one it needs.

use crate::{Harness, root, sys};
use anyhow::bail;
use std::path::PathBuf;

/// Refuse a file larger than this rather than pull it into guest memory.
/// A harness reads through its own heap, so an unbounded read is an
/// unbounded allocation inside the sandbox.
pub const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;

/// Read files, bounded by `root`.
pub fn read(root: PathBuf) -> Harness {
    sys::fs::read(move |path| {
        let path = root::resolve(&root, path)?;
        let size = std::fs::metadata(&path)
            .map(|meta| meta.len())
            .unwrap_or_default();
        if size > MAX_FILE_SIZE {
            bail!("file is too large ({size} bytes, max {MAX_FILE_SIZE})");
        }

        Ok(std::fs::read(&path)?)
    })
}

/// Write files, bounded by `root`.
pub fn write(root: PathBuf) -> Harness {
    sys::fs::write(move |path, content| {
        let path = root::resolve(&root, path)?;
        std::fs::write(&path, content)?;
        Ok(())
    })
}
