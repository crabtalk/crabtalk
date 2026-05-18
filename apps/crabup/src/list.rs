//! List crabtalk binaries with installed and running status.

use anyhow::{Context, Result};
use std::net::TcpStream;

use crate::registry::Entry;

/// Return the set of installed crabtalk-owned crates, sorted.
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

/// Return the port if the service is alive, `None` otherwise.
fn running_port(name: &str) -> Option<u16> {
    let port_file = wcore::paths::service_port_file(name);
    let port: u16 = std::fs::read_to_string(port_file)
        .ok()?
        .trim()
        .parse()
        .ok()?;
    TcpStream::connect(("127.0.0.1", port)).ok()?;
    Some(port)
}

struct Row {
    name: &'static str,
    state: &'static str,
    status: &'static str,
    port: String,
    sort_key: u8,
}

/// Print a unified list of available crabtalk binaries.
pub fn run() -> Result<()> {
    let installed_set: std::collections::HashSet<String> = installed()?.into_iter().collect();

    let mut rows: Vec<Row> = Entry::all()
        .iter()
        .map(|e| {
            let installed = installed_set.contains(e.krate);
            let serviceable = e.label.is_some();
            let port = running_port(e.short);
            let (state, status, port_str, sort_key) = match (installed, serviceable, port) {
                (true, true, Some(p)) => ("installed", "running", p.to_string(), 0),
                (true, true, None) => ("installed", "", String::new(), 1),
                (true, false, _) => ("installed", "-", "-".to_owned(), 2),
                (false, _, _) => ("", "", String::new(), 3),
            };
            Row {
                name: e.short,
                state,
                status,
                port: port_str,
                sort_key,
            }
        })
        .collect();

    rows.sort_by_key(|r| (r.sort_key, r.name));

    let nw = rows.iter().map(|r| r.name.len()).max().unwrap_or(0).max(4);
    let sw = rows.iter().map(|r| r.state.len()).max().unwrap_or(0).max(5);
    let tw = rows
        .iter()
        .map(|r| r.status.len())
        .max()
        .unwrap_or(0)
        .max(6);

    println!("{:<nw$}  {:<sw$}  {:<tw$}  PORT", "NAME", "STATE", "STATUS");
    for row in &rows {
        println!(
            "{:<nw$}  {:<sw$}  {:<tw$}  {port}",
            row.name,
            row.state,
            row.status,
            port = row.port,
        );
    }
    Ok(())
}
