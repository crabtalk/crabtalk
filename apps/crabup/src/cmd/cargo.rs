//! Thin wrappers over `cargo install` / `cargo uninstall`.

use anyhow::{Context, Result, bail};
use std::process::Command;

#[derive(Default)]
pub struct InstallOpts<'a> {
    pub version: Option<&'a str>,
    pub features: &'a [String],
    pub no_default_features: bool,
}

pub fn install(krate: &str, opts: InstallOpts<'_>) -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.args(["install", krate]);
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
