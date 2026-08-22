//! Report what is installed, and by which path it got there.

use anyhow::{Context, Result};

use crate::cmd;

/// True if the managed binary is in `~/.cargo/bin` rather than crabup's.
fn cargo_installed() -> Result<bool> {
    let path = dirs::home_dir()
        .context("could not resolve home directory")?
        .join(".cargo/.crates.toml");
    if !path.exists() {
        return Ok(false);
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let parsed: toml::Value =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    let Some(v1) = parsed.get("v1").and_then(|v| v.as_table()) else {
        return Ok(false);
    };
    Ok(v1
        .keys()
        .filter_map(|k| k.split_whitespace().next())
        .any(|krate| krate == cmd::AGENT))
}

pub fn run() -> Result<()> {
    let state = match (
        crate::dirs::BIN_DIR.join(cmd::AGENT).exists(),
        cargo_installed()?,
    ) {
        (true, _) => "installed",
        (false, true) => "cargo",
        _ => "not installed",
    };
    match cmd::manifest::version(cmd::AGENT) {
        Some(version) => println!("{}  {state}  {version}", cmd::AGENT),
        None => println!("{}  {state}", cmd::AGENT),
    }
    Ok(())
}
