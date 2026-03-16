//! `walrus daemon logs` — view daemon or service logs.
//!
//! Thin wrapper around `tail` — all extra flags are passed through.

use anyhow::{Context, Result};
use wcore::paths::LOGS_DIR;

/// Display log output by delegating to `tail`.
pub fn logs(service: Option<&str>, tail_args: &[String]) -> Result<()> {
    let filename = match service {
        Some(name) => format!("{name}.log"),
        None => "daemon.log".to_owned(),
    };
    let path = LOGS_DIR.join(&filename);
    if !path.exists() {
        anyhow::bail!("log file not found: {}", path.display());
    }

    let status = std::process::Command::new("tail")
        .args(tail_args)
        .arg(&path)
        .status()
        .context("failed to run tail")?;
    if !status.success() {
        anyhow::bail!("tail exited with {}", status);
    }
    Ok(())
}
