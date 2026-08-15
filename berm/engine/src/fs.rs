//! `fs` — files under the granted root.

use crate::{root, wire};
use anyhow::{Result, bail};
use std::path::Path;

/// Refuse a file larger than this rather than pull it into guest memory.
/// A harness reads through its own heap, so an unbounded read is an
/// unbounded allocation inside the sandbox.
pub const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;

/// Read a file whole. Request: `[path]`.
pub fn read(root: &Path, request: &[u8]) -> Result<Vec<u8>> {
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

/// Write a file, replacing what was there. Request: `[path, content]`.
pub fn write(root: &Path, request: &[u8]) -> Result<Vec<u8>> {
    let fields = wire::fields(request)?;
    let path = root::resolve(root, wire::text(&fields, 0, "path")?)?;
    let Some(content) = fields.get(1) else {
        bail!("request has no content");
    };

    std::fs::write(&path, content)?;
    Ok(Vec::new())
}
