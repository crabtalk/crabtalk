//! `walrus daemon` — daemon lifecycle management.

use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::Path;
use wcore::paths::{CONFIG_DIR, TCP_PORT_FILE};

/// Manage the walrus daemon.
#[derive(Args, Debug)]
pub struct Daemon {
    #[command(subcommand)]
    pub command: DaemonCommand,
}

#[derive(Subcommand, Debug)]
pub enum DaemonCommand {
    /// Start the daemon in the foreground.
    Start,
    /// Trigger a hot-reload of the running daemon.
    Reload,
    /// Install walrus as a system service (launchd/systemd).
    Install,
    /// Uninstall the walrus system service.
    Uninstall,
}

impl Daemon {
    pub async fn run(self, socket_path: &Path) -> Result<()> {
        match self.command {
            DaemonCommand::Start => start().await,
            DaemonCommand::Reload => reload(socket_path).await,
            DaemonCommand::Install => install(),
            DaemonCommand::Uninstall => uninstall(),
        }
    }
}

// ── Start ────────────────────────────────────────────────────────────

async fn start() -> Result<()> {
    daemon::config::scaffold_config_dir(&CONFIG_DIR)?;

    // Check if providers are configured; prompt if empty.
    let config_path = CONFIG_DIR.join("walrus.toml");
    let config = daemon::DaemonConfig::load(&config_path)?;
    if config.provider.is_empty() {
        setup_provider(&config_path)?;
    }

    let handle = daemon::Daemon::start(&CONFIG_DIR).await?;

    // UDS transport.
    let (socket_path, socket_join) = daemon::setup_socket(&handle.shutdown_tx, &handle.event_tx)?;
    tracing::info!("walrusd listening on {}", socket_path.display());

    // TCP transport.
    let (tcp_join, tcp_port) = daemon::setup_tcp(&handle.shutdown_tx, &handle.event_tx)?;
    std::fs::write(&*TCP_PORT_FILE, tcp_port.to_string())?;
    tracing::info!("wrote tcp port file at {}", TCP_PORT_FILE.display());

    handle.wait_until_ready().await?;

    tokio::signal::ctrl_c().await?;
    tracing::info!("received ctrl-c, shutting down");

    let grace = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        handle.shutdown().await?;
        socket_join.await?;
        tcp_join.await?;
        anyhow::Ok(())
    });
    if grace.await.is_err() {
        tracing::warn!("graceful shutdown timed out, forcing exit");
    }
    let _ = std::fs::remove_file(socket_path);
    let _ = std::fs::remove_file(&*TCP_PORT_FILE);
    tracing::info!("walrusd shut down");
    std::process::exit(0)
}

// ── Provider setup prompt ────────────────────────────────────────────

/// Interactive provider setup for first-time daemon start.
pub(crate) fn setup_provider(config_path: &Path) -> Result<()> {
    use crate::cmd::model::PRESETS;
    use std::io::{BufRead, Write};
    use toml_edit::{Array, DocumentMut, Item, Table, value};

    println!("\nNo providers configured. Let's set one up.\n");
    println!("Select a provider:");
    for (i, preset) in PRESETS.iter().enumerate() {
        println!("  [{}] {}", i + 1, preset.name);
    }
    print!("\nChoice [1-{}]: ", PRESETS.len());
    std::io::stdout().flush()?;

    let stdin = std::io::stdin();
    let choice_line = stdin.lock().lines().next().unwrap_or(Ok(String::new()))?;
    let idx: usize = choice_line
        .trim()
        .parse::<usize>()
        .unwrap_or(0)
        .saturating_sub(1);
    if idx >= PRESETS.len() {
        anyhow::bail!("invalid choice");
    }
    let preset = &PRESETS[idx];

    // API key (skip for ollama).
    let api_key = if preset.name != "ollama" {
        print!("API key: ");
        std::io::stdout().flush()?;
        let key = stdin.lock().lines().next().unwrap_or(Ok(String::new()))?;
        let key = key.trim().to_string();
        if key.is_empty() {
            anyhow::bail!("API key is required for {}", preset.name);
        }
        Some(key)
    } else {
        None
    };

    // Base URL (use preset default or ask for custom).
    let base_url = if preset.name == "custom" {
        print!("Base URL: ");
        std::io::stdout().flush()?;
        let url = stdin.lock().lines().next().unwrap_or(Ok(String::new()))?;
        url.trim().to_string()
    } else {
        preset.base_url.to_string()
    };

    // Model name.
    let default_model = default_model_for(preset.name);
    print!("Model name [{}]: ", default_model);
    std::io::stdout().flush()?;
    let model_line = stdin.lock().lines().next().unwrap_or(Ok(String::new()))?;
    let model = model_line.trim();
    let model = if model.is_empty() {
        default_model
    } else {
        model
    };

    // Write to walrus.toml.
    let content = std::fs::read_to_string(config_path)?;
    let mut doc: DocumentMut = content.parse()?;

    // Set [walrus] model.
    if !doc.contains_key("walrus") {
        doc.insert("walrus", Item::Table(Table::new()));
    }
    doc["walrus"]["model"] = value(model);

    // Set [provider.<name>].
    if !doc.contains_key("provider") {
        doc.insert("provider", Item::Table(Table::new()));
    }
    let provider_table = doc["provider"].as_table_mut().unwrap();
    let mut entry = Table::new();
    if let Some(ref key) = api_key {
        entry.insert("api_key", value(key.as_str()));
    }
    if !base_url.is_empty() {
        entry.insert("base_url", value(base_url.as_str()));
    }
    entry.insert("standard", value(preset.standard));
    let mut models = Array::new();
    models.push(model);
    entry.insert("models", value(models));
    provider_table.insert(preset.name, Item::Table(entry));

    std::fs::write(config_path, doc.to_string())?;
    println!("\nSaved to {}\n", config_path.display());
    Ok(())
}

fn default_model_for(provider: &str) -> &str {
    match provider {
        "anthropic" => "claude-sonnet-4-5-20250514",
        "openai" => "gpt-4o",
        "deepseek" => "deepseek-chat",
        "ollama" => "llama3",
        _ => "default",
    }
}

// ── Reload ───────────────────────────────────────────────────────────

async fn reload(socket_path: &Path) -> Result<()> {
    let mut runner = crate::repl::runner::Runner::connect(socket_path).await?;
    runner.reload().await?;
    println!("daemon reloaded");
    Ok(())
}

// ── Service install/uninstall ────────────────────────────────────────

#[cfg(target_os = "macos")]
const LAUNCHD_TEMPLATE: &str = include_str!("launchd.plist");
#[cfg(target_os = "linux")]
const SYSTEMD_TEMPLATE: &str = include_str!("systemd.service");

/// Render a template by replacing `{binary}` and `{log_dir}` placeholders.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn render_template(template: &str, binary: &Path, log_dir: &Path) -> String {
    template
        .replace("{binary}", &binary.display().to_string())
        .replace("{log_dir}", &log_dir.display().to_string())
}

#[cfg(target_os = "macos")]
fn install() -> Result<()> {
    let binary = std::env::current_exe()?;
    let plist = render_template(LAUNCHD_TEMPLATE, &binary, &CONFIG_DIR);

    let plist_path = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?
        .join("Library/LaunchAgents/xyz.openwalrus.walrus.plist");

    if let Some(parent) = plist_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&plist_path, plist)?;
    println!("wrote {}", plist_path.display());

    let status = std::process::Command::new("launchctl")
        .args(["load", "-w"])
        .arg(&plist_path)
        .status()?;
    if status.success() {
        println!("service loaded and started");
    } else {
        anyhow::bail!("launchctl load failed (exit {})", status);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn uninstall() -> Result<()> {
    let plist_path = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?
        .join("Library/LaunchAgents/xyz.openwalrus.walrus.plist");

    if !plist_path.exists() {
        anyhow::bail!("service not installed ({})", plist_path.display());
    }

    let status = std::process::Command::new("launchctl")
        .args(["unload", "-w"])
        .arg(&plist_path)
        .status()?;
    if !status.success() {
        eprintln!("warning: launchctl unload exited with {}", status);
    }

    std::fs::remove_file(&plist_path)?;
    println!("service uninstalled");
    Ok(())
}

#[cfg(target_os = "linux")]
fn install() -> Result<()> {
    let binary = std::env::current_exe()?;
    let unit = render_template(SYSTEMD_TEMPLATE, &binary, &CONFIG_DIR);

    let unit_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?
        .join(".config/systemd/user");
    std::fs::create_dir_all(&unit_dir)?;

    let unit_path = unit_dir.join("walrus-daemon.service");
    std::fs::write(&unit_path, unit)?;
    println!("wrote {}", unit_path.display());

    let status = std::process::Command::new("systemctl")
        .args(["--user", "enable", "--now", "walrus-daemon.service"])
        .status()?;
    if status.success() {
        println!("service enabled and started");
    } else {
        anyhow::bail!("systemctl enable failed (exit {})", status);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn uninstall() -> Result<()> {
    let unit_path = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?
        .join(".config/systemd/user/walrus-daemon.service");

    if !unit_path.exists() {
        anyhow::bail!("service not installed ({})", unit_path.display());
    }

    let status = std::process::Command::new("systemctl")
        .args(["--user", "disable", "--now", "walrus-daemon.service"])
        .status()?;
    if !status.success() {
        eprintln!("warning: systemctl disable exited with {}", status);
    }

    std::fs::remove_file(&unit_path)?;
    println!("service uninstalled");
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn install() -> Result<()> {
    anyhow::bail!("service install is only supported on macOS and Linux")
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn uninstall() -> Result<()> {
    anyhow::bail!("service uninstall is only supported on macOS and Linux")
}
