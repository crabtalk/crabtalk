//! CLI argument parsing and command dispatch.

use crate::repl::runner::Runner;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use compact_str::CompactString;
use std::path::PathBuf;
use wcore::paths::TCP_PORT_FILE;

pub mod attach;
pub mod auth;
pub mod console;
pub mod daemon;
pub mod hub;

/// Walrus CLI client — connects to walrusd via Unix domain socket.
#[derive(Parser, Debug)]
#[command(name = "walrus", about = "Walrus CLI client")]
pub struct Cli {
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Command,

    /// Agent name override.
    #[arg(long, global = true)]
    pub agent: Option<CompactString>,

    /// Path to the walrusd socket.
    #[arg(long, global = true)]
    pub socket: Option<PathBuf>,
}

impl Cli {
    /// Resolve the agent name from CLI flags or fall back to "assistant".
    pub fn resolve_agent(&self) -> CompactString {
        self.agent.clone().unwrap_or_else(|| "walrus".into())
    }

    /// Resolve the socket path from CLI flag or default.
    fn resolve_socket(&self) -> PathBuf {
        self.socket
            .clone()
            .unwrap_or_else(|| wcore::paths::SOCKET_PATH.clone())
    }

    /// Parse and dispatch the CLI command.
    pub async fn run(self) -> Result<()> {
        let agent = self.resolve_agent();
        let socket_path = self.resolve_socket();
        match self.command {
            Command::Auth(cmd) => cmd.run(),
            Command::Attach(cmd) => {
                attach::ensure_providers(&socket_path).await?;
                let runner = connect(cmd.tcp, &socket_path).await?;
                cmd.run(runner, agent).await
            }
            Command::Console(cmd) => {
                let runner = connect_uds(&socket_path).await?;
                cmd.run(runner).await
            }
            Command::Hub(cmd) => {
                let mut runner = connect_uds(&socket_path).await?;
                cmd.run(&mut runner).await
            }
            Command::Daemon(cmd) => cmd.run(&socket_path).await,
        }
    }
}

/// Top-level subcommands.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Attach to an agent via the interactive chat REPL.
    Attach(attach::Attach),
    /// Configure providers, models, and channel tokens interactively.
    Auth(auth::Auth),
    /// Interactive console for sessions and tasks.
    Console(console::Console),
    /// Install or uninstall hub packages.
    Hub(hub::Hub),
    /// Manage the walrus daemon (start, reload, install, uninstall).
    Daemon(daemon::Daemon),
}

/// Connect to walrusd via TCP or UDS.
async fn connect(use_tcp: bool, socket_path: &std::path::Path) -> Result<Runner> {
    if use_tcp {
        connect_tcp().await
    } else {
        connect_uds(socket_path).await
    }
}

/// Connect to walrusd via TCP, reading the port from the port file.
async fn connect_tcp() -> Result<Runner> {
    let port_str = std::fs::read_to_string(&*TCP_PORT_FILE).with_context(|| {
        format!(
            "failed to read TCP port file at {}. Is walrusd running?",
            TCP_PORT_FILE.display()
        )
    })?;
    let port: u16 = port_str
        .trim()
        .parse()
        .with_context(|| format!("invalid port in {}", TCP_PORT_FILE.display()))?;
    Runner::connect_tcp(port).await.with_context(|| {
        format!("failed to connect to walrusd via TCP on port {port}. Is walrusd running?")
    })
}

/// Connect to walrusd via Unix domain socket.
async fn connect_uds(socket_path: &std::path::Path) -> Result<Runner> {
    Runner::connect(socket_path).await.with_context(|| {
        format!(
            "failed to connect to walrusd at {}. Is walrusd running?",
            socket_path.display()
        )
    })
}
