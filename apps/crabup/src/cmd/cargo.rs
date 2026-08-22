//! Thin wrappers over `cargo install` / `cargo uninstall`.

use anyhow::{Context, Result, bail};
use std::process::Command;

/// Where a nightly comes from: the branch development happens on, built
/// against the lockfile that branch tested with.
const REPO: &str = "https://github.com/crabtalk/crabtalk";
const NIGHTLY_BRANCH: &str = "dev";

#[derive(Default)]
pub struct InstallOpts<'a> {
    pub version: Option<&'a str>,
    pub features: &'a [String],
    pub no_default_features: bool,
    pub nightly: bool,
}

pub fn install(krate: &str, opts: InstallOpts<'_>) -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.args(["install", krate]);
    if opts.nightly {
        cmd.args(["--git", REPO, "--branch", NIGHTLY_BRANCH, "--locked"]);
    }
    if let Some(v) = opts.version {
        cmd.args(["--version", v]);
    }
    if !opts.features.is_empty() {
        cmd.args(["--features", &opts.features.join(",")]);
    }
    if opts.no_default_features {
        cmd.arg("--no-default-features");
    }
    let status = cmd
        .status()
        .context("failed to run `cargo` — install Rust from https://rustup.rs")?;
    if !status.success() {
        bail!("cargo install {krate} failed");
    }
    Ok(())
}

pub fn uninstall(krate: &str) -> Result<()> {
    let status = Command::new("cargo")
        .args(["uninstall", krate])
        .status()
        .context("failed to run `cargo` — install Rust from https://rustup.rs")?;
    if !status.success() {
        bail!("cargo uninstall {krate} failed");
    }
    Ok(())
}
