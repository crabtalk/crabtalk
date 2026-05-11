//! List running crabtalk services by scanning `~/.crabtalk/run/*.port`.

use anyhow::Result;
use std::net::TcpStream;

pub fn run() -> Result<()> {
    let run_dir = &*wcore::paths::RUN_DIR;
    let Ok(entries) = std::fs::read_dir(run_dir) else {
        println!("no services running");
        return Ok(());
    };

    let mut rows: Vec<(String, u16, &'static str)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("port") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(port) = contents.trim().parse::<u16>() else {
            continue;
        };
        let status = if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            "running"
        } else {
            "stale"
        };
        rows.push((name.to_string(), port, status));
    }

    if rows.is_empty() {
        println!("no services running");
        return Ok(());
    }

    rows.sort();

    let name_w = rows
        .iter()
        .map(|(n, _, _)| n.len())
        .max()
        .unwrap_or(0)
        .max("NAME".len());
    let port_w = rows
        .iter()
        .map(|(_, p, _)| p.to_string().len())
        .max()
        .unwrap_or(0)
        .max("PORT".len());

    println!(
        "{:<name_w$}  {:>port_w$}  STATUS",
        "NAME",
        "PORT",
        name_w = name_w,
        port_w = port_w,
    );
    for (name, port, status) in rows {
        println!(
            "{name:<name_w$}  {port:>port_w$}  {status}",
            name_w = name_w,
            port_w = port_w,
        );
    }
    Ok(())
}
