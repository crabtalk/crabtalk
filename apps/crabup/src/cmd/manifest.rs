//! Track installed binaries and their versions in `~/.crabtalk/installed.toml`.

use anyhow::{Context, Result};

fn manifest_path() -> std::path::PathBuf {
    crate::dirs::CONFIG_DIR.join("installed.toml")
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
pub fn record(bin: &str, version: &str) -> Result<()> {
    let mut table = load()?;
    table.insert(bin.to_string(), toml::Value::String(version.to_string()));
    save(&table)
}

/// Remove an entry from the manifest.
pub fn remove(bin: &str) -> Result<()> {
    let mut table = load()?;
    table.remove(bin);
    save(&table)
}

/// Get the installed version of a binary, if tracked.
pub fn version(bin: &str) -> Option<String> {
    load().ok()?.get(bin)?.as_str().map(String::from)
}
