//! External command launcher with auto-install.
//!
//! When `crabtalk <name>` is called with an unrecognized subcommand, this
//! module resolves `crabtalk-<name>`, auto-installs from crates.io if missing,
//! and forwards all arguments directly.

use anyhow::{Context, Result, bail};
use std::{ffi::OsString, path::PathBuf, process::Command};

/// Resolve and launch an external `crabtalk-<name>` binary.
pub fn run(args: Vec<OsString>) -> Result<()> {
    let name = args
        .first()
        .ok_or_else(|| anyhow::anyhow!("no subcommand provided"))?
        .to_string_lossy()
        .to_string();
    let bin_name = format!("crabtalk-{name}");

    let binary = match find_binary(&bin_name) {
        Some(path) => path,
        None => {
            eprintln!("installing {bin_name} from crates.io...");
            let status = Command::new("cargo")
                .args(["install", &bin_name])
                .status()
                .context("failed to run cargo install")?;
            if !status.success() {
                bail!("package crabtalk-{name} not found");
            }
            find_binary(&bin_name)
                .ok_or_else(|| anyhow::anyhow!("{bin_name} not found after install"))?
        }
    };

    let status = Command::new(&binary)
        .args(&args[1..])
        .status()
        .with_context(|| format!("failed to run {}", binary.display()))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// Look for an external binary next to the current exe, then on PATH.
fn find_binary(name: &str) -> Option<PathBuf> {
    if let Ok(current) = std::env::current_exe()
        && let Some(dir) = current.parent()
    {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    let path = std::env::var("PATH").unwrap_or_default();
    for dir in path.split(':') {
        let candidate = PathBuf::from(dir).join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}
