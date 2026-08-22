//! The boundary `fs` and `exec` are granted within.
//!
//! A harness names a root in its declaration and every path it offers is
//! resolved inside it. This is the whole of the confinement: the address space
//! keeps a harness from reading the host's memory, and this keeps it from
//! reading the host's disk.
//!
//! `..` is resolved lexically *before* the filesystem is consulted, because
//! `root.join("../etc/passwd")` is a path that starts with the root right up
//! until something interprets it. Symlinks are caught after, by canonicalising
//! what exists — a link inside the root pointing out of it is the case a purely
//! lexical check misses.

use anyhow::{Result, bail};
use std::path::{Component, Path, PathBuf};

/// Resolve `path` within `root`, refusing anything that leaves it.
pub fn resolve(root: &Path, path: &str) -> Result<PathBuf> {
    let root = root
        .canonicalize()
        .unwrap_or_else(|_| normalize(&PathBuf::from(root)));

    let joined = match Path::new(path) {
        absolute if absolute.is_absolute() => absolute.to_path_buf(),
        relative => root.join(relative),
    };

    let resolved = settle(&normalize(&joined));
    if !resolved.starts_with(&root) {
        bail!("{path} is outside the harness root");
    }
    Ok(resolved)
}

/// Resolve `.` and `..` textually. A path need not exist to be normalized,
/// which is what makes this usable for a file about to be created.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Canonicalise the longest ancestor that exists and re-attach the rest, so a
/// symlink anywhere along the path is followed even when the leaf is missing.
fn settle(path: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut existing = path;
    loop {
        if let Ok(real) = existing.canonicalize() {
            let mut settled = real;
            for name in missing.iter().rev() {
                settled.push(name);
            }
            return settled;
        }
        let Some(parent) = existing.parent() else {
            return path.to_path_buf();
        };
        let Some(name) = existing.file_name() else {
            return path.to_path_buf();
        };
        missing.push(name.to_owned());
        existing = parent;
    }
}
