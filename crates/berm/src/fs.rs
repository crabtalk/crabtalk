//! `fs` — files under a granted root.
//!
//! Read and write are separate system harnesses, so a harness that summarises a
//! directory can be given the one it needs.
//!
//! Searching happens here rather than in the guest for the reason the guest is
//! `no_std` at all: matching a tree from inside the sandbox would pull every
//! candidate file's bytes across the syscall boundary to find a dozen lines.

use crate::{root, sys};
use anyhow::bail;
use berm::Harness;
use globset::GlobBuilder;
use grep_regex::RegexMatcher;
use grep_searcher::{BinaryDetection, SearcherBuilder, sinks};
use ignore::{WalkBuilder, overrides::OverrideBuilder};
use std::{
    cmp::Reverse,
    path::{Path, PathBuf},
    time::SystemTime,
};

/// Refuse a file larger than this rather than pull it into guest memory.
/// A harness reads through its own heap, so an unbounded read is an
/// unbounded allocation inside the sandbox.
pub const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;

/// Results one search may return. A result is a line and the whole set has to
/// fit in one invocation's buffer, which the OS harness sizes for two thousand.
const MAX_RESULTS: usize = 2000;

/// Paths only, and the mode a search that names none gets.
const FILES: &str = "files";
/// Matching lines, prefixed by path and line number.
const CONTENT: &str = "content";
/// Match counts per file.
const COUNT: &str = "count";

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

/// Match paths against a glob, bounded by `root`.
pub fn glob(root: PathBuf) -> Harness {
    sys::fs::glob(move |pattern, path| {
        let anchor = root::resolve(&root, ".")?;
        let base = root::resolve(&root, path)?;
        let matcher = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()?
            .compile_matcher();

        let mut hits = Vec::new();
        for entry in walker(&base).build().flatten() {
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            // The pattern is matched against the path below `base` so `**/*.rs`
            // means what it says under a named subdirectory, but what comes back
            // is relative to the root, because that is what `read` takes.
            let Ok(relative) = entry.path().strip_prefix(&base) else {
                continue;
            };
            if !matcher.is_match(relative) {
                continue;
            }
            let modified = entry
                .metadata()
                .ok()
                .and_then(|meta| meta.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            hits.push((modified, display(&anchor, entry.path())));
        }

        hits.sort_unstable_by_key(|(modified, _)| Reverse(*modified));
        hits.truncate(MAX_RESULTS);
        Ok(join(hits.into_iter().map(|(_, path)| path)))
    })
}

/// Search file contents, bounded by `root`.
pub fn grep(root: PathBuf) -> Harness {
    sys::fs::grep(move |pattern, path, include, mode| {
        if !matches!(mode, FILES | CONTENT | COUNT) {
            bail!("unknown mode {mode} (expected {FILES}, {CONTENT} or {COUNT})");
        }

        let anchor = root::resolve(&root, ".")?;
        let base = root::resolve(&root, path)?;
        let matcher = RegexMatcher::new_line_matcher(pattern)?;

        // `include` filters the walk the way ripgrep's `--glob` does, so a
        // pattern naming no directory matches at any depth: `*.rs`, not `**/*.rs`.
        let mut walk = walker(&base);
        if !include.is_empty() {
            let mut overrides = OverrideBuilder::new(&base);
            overrides.add(include)?;
            walk.overrides(overrides.build()?);
        }

        // Line numbers stay on in every mode: `sinks::UTF8` fails the whole
        // search without them, and only `content` ever prints one.
        let mut searcher = SearcherBuilder::new()
            .line_number(true)
            .binary_detection(BinaryDetection::quit(b'\x00'))
            .build();

        let mut out: Vec<String> = Vec::new();
        for entry in walk.build().flatten() {
            if out.len() >= MAX_RESULTS {
                break;
            }
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let path = entry.path();
            let shown = display(&anchor, path);
            // A file that cannot be read is skipped rather than fatal: one
            // unreadable path should not cost the whole search.
            match mode {
                FILES => {
                    let mut hit = false;
                    let _ = searcher.search_path(
                        &matcher,
                        path,
                        sinks::UTF8(|_, _| {
                            hit = true;
                            Ok(false)
                        }),
                    );
                    if hit {
                        out.push(shown);
                    }
                }
                COUNT => {
                    let mut count = 0usize;
                    let _ = searcher.search_path(
                        &matcher,
                        path,
                        sinks::UTF8(|_, _| {
                            count += 1;
                            Ok(true)
                        }),
                    );
                    if count > 0 {
                        out.push(format!("{shown}:{count}"));
                    }
                }
                _ => {
                    let _ = searcher.search_path(
                        &matcher,
                        path,
                        sinks::UTF8(|number, line| {
                            out.push(format!("{shown}:{number}:{}", line.trim_end()));
                            Ok(out.len() < MAX_RESULTS)
                        }),
                    );
                }
            }
        }

        Ok(join(out.into_iter()))
    })
}

/// A walk that skips what `.gitignore` names whether or not the root is a
/// checkout — a harness root is as often a session directory as a repository,
/// and which one it happens to be should not change what a search returns.
fn walker(base: &Path) -> WalkBuilder {
    let mut walk = WalkBuilder::new(base);
    walk.require_git(false);
    walk
}

/// A path as the harness names it: relative to the root everything is bounded by.
fn display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// One result per line, and nothing at all when there are none — an empty
/// result says "no matches" without a sentence the model has to read.
fn join(results: impl Iterator<Item = String>) -> Vec<u8> {
    results.collect::<Vec<_>>().join("\n").into_bytes()
}
