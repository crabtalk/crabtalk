//! List crabtalk binaries with installed status.

use anyhow::{Context, Result};

use crate::registry::Entry;

/// Return the set of installed crabtalk-owned crates (from cargo), sorted.
pub fn installed() -> Result<Vec<String>> {
    let path = dirs::home_dir()
        .context("could not resolve home directory")?
        .join(".cargo/.crates.toml");
    if !path.exists() {
        return Ok(vec![]);
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let parsed: toml::Value =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    let Some(v1) = parsed.get("v1").and_then(|v| v.as_table()) else {
        return Ok(vec![]);
    };

    let mut names: Vec<String> = v1
        .keys()
        .filter_map(|k| {
            let krate = k.split_whitespace().next()?;
            Entry::is_crabtalk(krate).then(|| krate.to_string())
        })
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

struct Row {
    name: &'static str,
    state: String,
    version: String,
}

/// Print a unified list of available crabtalk binaries.
pub fn run() -> Result<()> {
    let cargo_set: std::collections::HashSet<String> = installed()?.into_iter().collect();
    let manifest = crate::manifest::all().unwrap_or_default();

    let mut rows: Vec<Row> = Entry::all()
        .iter()
        .map(|e| {
            let managed = wcore::paths::BIN_DIR.join(e.bin).exists();
            let cargo = cargo_set.contains(e.krate);

            let state = match (managed, cargo) {
                (true, _) => "installed".to_string(),
                (false, true) => "cargo".to_string(),
                _ => String::new(),
            };

            let version = manifest.get(e.short).cloned().unwrap_or_default();

            Row {
                name: e.short,
                state,
                version,
            }
        })
        .collect();

    rows.sort_by_key(|r| (r.state.is_empty(), r.name));

    let nw = rows.iter().map(|r| r.name.len()).max().unwrap_or(0).max(4);
    let sw = rows.iter().map(|r| r.state.len()).max().unwrap_or(0).max(5);

    println!("{:<nw$}  {:<sw$}  VERSION", "NAME", "STATE");
    for row in &rows {
        println!("{:<nw$}  {:<sw$}  {}", row.name, row.state, row.version);
    }
    Ok(())
}
