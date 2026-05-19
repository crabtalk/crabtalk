//! Track installed binaries and their versions in `~/.crabtalk/installed.toml`.

use anyhow::{Context, Result};

fn manifest_path() -> std::path::PathBuf {
    wcore::paths::CONFIG_DIR.join("installed.toml")
}

fn load() -> Result<toml::Table> {
    let path = manifest_path();
    if !path.exists() {
        return Ok(toml::Table::new());
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    content
        .parse::<toml::Table>()
        .with_context(|| format!("failed to parse {}", path.display()))
}

fn save(table: &toml::Table) -> Result<()> {
    let path = manifest_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, table.to_string())
        .with_context(|| format!("failed to write {}", path.display()))
}

/// Record an installed binary's version.
pub fn record(short: &str, version: &str) -> Result<()> {
    let mut table = load()?;
    table.insert(short.to_string(), toml::Value::String(version.to_string()));
    save(&table)
}

/// Remove an entry from the manifest.
pub fn remove(short: &str) -> Result<()> {
    let mut table = load()?;
    table.remove(short);
    save(&table)
}

/// Get the installed version of a binary, if tracked.
pub fn version(short: &str) -> Option<String> {
    load().ok()?.get(short)?.as_str().map(String::from)
}

/// All installed entries: short name → version.
pub fn all() -> Result<std::collections::BTreeMap<String, String>> {
    let table = load()?;
    Ok(table
        .into_iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
        .collect())
}
