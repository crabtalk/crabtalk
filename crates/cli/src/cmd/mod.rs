//! CLI argument parsing and command dispatch.

use crate::repl::runner::Runner;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use wcore::paths::TCP_PORT_FILE;

pub mod attach;
pub mod auth;
pub mod console;
pub mod daemon;
pub mod gateway;
pub mod hub;

/// Crabtalk CLI client — connects to crabtalk daemon via Unix domain socket.
#[derive(Parser, Debug)]
#[command(name = "crabtalk", about = "Crabtalk CLI client")]
pub struct Cli {
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Command,

    /// Agent name override.
    #[arg(long, global = true)]
    pub agent: Option<String>,

    /// Path to the crabtalk daemon socket.
    #[arg(long, global = true)]
    pub socket: Option<PathBuf>,
}

impl Cli {
    /// Build a `RUST_LOG`-style filter string from the `-v` count on `daemon run`.
    ///
    /// Returns `None` when no `-v` flag is present (fall back to `RUST_LOG` env).
    pub fn log_filter(&self) -> Option<&'static str> {
        if let Command::Daemon(ref d) = self.command
            && let daemon::DaemonCommand::Run { verbose } = d.command
            && verbose > 0
        {
            Some(match verbose {
                1 => "crabtalk=info",
                2 => "crabtalk=debug",
                _ => "crabtalk=trace",
            })
        } else {
            None
        }
    }

    /// Resolve the agent name from CLI flags or fall back to "assistant".
    pub fn resolve_agent(&self) -> String {
        self.agent.clone().unwrap_or_else(|| "crab".to_owned())
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
            Command::Gateway(cmd) => cmd.run(),
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
    /// Manage the crabtalk daemon (run, start, stop, reload).
    Daemon(daemon::Daemon),
    /// Manage gateways (telegram start/stop). Forwards to crabtalk-gateway.
    Gateway(gateway::Gateway),
}

/// Connect to crabtalk daemon via TCP or UDS.
async fn connect(use_tcp: bool, socket_path: &std::path::Path) -> Result<Runner> {
    if use_tcp {
        connect_tcp().await
    } else {
        connect_uds(socket_path).await
    }
}

/// Connect to crabtalk daemon via TCP, reading the port from the port file.
async fn connect_tcp() -> Result<Runner> {
    let port_str = std::fs::read_to_string(&*TCP_PORT_FILE).with_context(|| {
        format!(
            "failed to read TCP port file at {}. Is crabtalk daemon running?",
            TCP_PORT_FILE.display()
        )
    })?;
    let port: u16 = port_str
        .trim()
        .parse()
        .with_context(|| format!("invalid port in {}", TCP_PORT_FILE.display()))?;
    Runner::connect_tcp(port).await.with_context(|| {
        format!("failed to connect to crabtalk daemon via TCP on port {port}. Is crabtalk daemon running?")
    })
}

/// Connect to crabtalk daemon via Unix domain socket.
async fn connect_uds(socket_path: &std::path::Path) -> Result<Runner> {
    Runner::connect(socket_path).await.with_context(|| {
        format!(
            "failed to connect to crabtalk daemon at {}. Is crabtalk daemon running?",
            socket_path.display()
        )
    })
}
