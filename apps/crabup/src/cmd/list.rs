//! What is installed, read from cargo's own record.

use anyhow::{Context, Result};

use crate::cmd::CRATES;

/// Installed versions, keyed by crate, from `~/.cargo/.crates.toml`.
///
/// There is no parallel state file. If cargo's record is wrong then cargo
/// is wrong, and crabup being wrong with it is the correct behaviour.
fn installed() -> Result<std::collections::BTreeMap<String, String>> {
    let path = dirs::home_dir()
        .context("could not resolve home directory")?
        .join(".cargo/.crates.toml");
    if !path.exists() {
        return Ok(Default::default());
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let parsed: toml::Value =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    let Some(v1) = parsed.get("v1").and_then(|v| v.as_table()) else {
        return Ok(Default::default());
    };

    // Keys read `name version (source)`.
    Ok(v1
        .keys()
        .filter_map(|k| {
            let mut parts = k.split_whitespace();
            Some((parts.next()?.to_owned(), parts.next()?.to_owned()))
        })
        .collect())
}

pub fn run() -> Result<()> {
    let installed = installed()?;
    let width = CRATES.iter().map(|(c, _)| c.len()).max().unwrap_or(0);
    for (krate, bin) in CRATES.iter().copied() {
        match installed.get(krate) {
            Some(version) => println!("{krate:<width$}  {version:<10}  {bin}"),
            None => println!("{krate:<width$}  {:<10}  {bin}", "-"),
        }
    }
    Ok(())
}
