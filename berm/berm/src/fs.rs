//! `fs` — files under a granted root.
//!
//! Read and write are separate system harnesses, so a harness that summarises a
//! directory can be given the one it needs.

use crate::{Harness, root, wire};
use anyhow::{Result, bail};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// Read a file. Request: `[path]`.
pub const READ: &str = "berm.fs.read";
/// Write a file. Request: `[path, content]`.
pub const WRITE: &str = "berm.fs.write";

/// Refuse a file larger than this rather than pull it into guest memory.
/// A harness reads through its own heap, so an unbounded read is an
/// unbounded allocation inside the sandbox.
pub const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;

/// Read files, bounded by `root`.
pub fn read(root: PathBuf) -> Harness {
    Harness {
        name: READ.to_owned(),
        call: Arc::new(move |request| read_at(&root, request)),
    }
}

/// Write files, bounded by `root`.
pub fn write(root: PathBuf) -> Harness {
    Harness {
        name: WRITE.to_owned(),
        call: Arc::new(move |request| write_at(&root, request)),
    }
}

fn read_at(root: &Path, request: &[u8]) -> Result<Vec<u8>> {
    let fields = wire::fields(request)?;
    let path = root::resolve(root, wire::text(&fields, 0, "path")?)?;

    let size = std::fs::metadata(&path)
        .map(|meta| meta.len())
        .unwrap_or_default();
    if size > MAX_FILE_SIZE {
        bail!("file is too large ({size} bytes, max {MAX_FILE_SIZE})");
    }

    Ok(std::fs::read(&path)?)
}

fn write_at(root: &Path, request: &[u8]) -> Result<Vec<u8>> {
    let fields = wire::fields(request)?;
    let path = root::resolve(root, wire::text(&fields, 0, "path")?)?;
    let Some(content) = fields.get(1) else {
        bail!("request has no content");
    };

    std::fs::write(&path, content)?;
    Ok(Vec::new())
}
