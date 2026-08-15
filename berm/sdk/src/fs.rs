//! Files, within the root the harness was granted.
//!
//! The root is not visible here and cannot be: it lives in the declaration and
//! is enforced host-side, so a path that escapes it comes back as an error
//! rather than as something this crate had to remember to check.

use crate::{
    abi::{HOST_FS_READ, HOST_FS_WRITE},
    cap, wire,
};
use alloc::{string::String, vec::Vec};

/// Read a file whole.
pub fn read(path: &str) -> Result<Vec<u8>, String> {
    cap::call(HOST_FS_READ, &wire::request(&[path.as_bytes()]))
}

/// Write a file, replacing what was there.
pub fn write(path: &str, content: &[u8]) -> Result<(), String> {
    cap::call(HOST_FS_WRITE, &wire::request(&[path.as_bytes(), content])).map(drop)
}
