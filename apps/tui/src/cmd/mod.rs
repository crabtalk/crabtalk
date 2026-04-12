//! CLI argument parsing and command dispatch.

use crate::repl::runner::Runner;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

pub mod config;
pub mod console;

/// Crabtalk TUI — interactive agent client.
#[derive(Parser, Debug)]
#[command(
    name = "crabtalk-tui",
    about = "Crabtalk TUI — interactive agent client"
)]
pub struct Cli {
    /// Connect via TCP instead of Unix domain socket.
    #[arg(long)]
    pub tcp: bool,
    /// Agent to use.
    #[arg(long, default_value = "crab")]
    pub agent: String,
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Option<Command>,
}

impl Cli {
    /// Parse and dispatch the CLI command.
    pub async fn run(self) -> Result<()> {
        match self.command {
            None => {
                let runner = connect(self.tcp).await?;
                let mut repl = crate::repl::ChatRepl::new(runner, self.agent)?;
                repl.run().await
            }
            Some(Command::Resume { file }) => {
                let runner = connect(self.tcp).await?;
                if let Some(path) = file {
                    let mut repl = crate::repl::ChatRepl::new(runner, self.agent)?;
                    repl.resume(std::path::PathBuf::from(path)).await
                } else {
                    let cmd = console::Console;
                    let selected = cmd.run(runner).await?;
                    if let Some(path) = selected {
                        let runner = connect(self.tcp).await?;
                        let mut repl = crate::repl::ChatRepl::new(runner, self.agent)?;
                        repl.resume(path).await
                    } else {
                        Ok(())
                    }
                }
            }
            Some(Command::Config(cmd)) => cmd.run().await,
        }
    }
}

/// Top-level subcommands.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Configure providers, models, and MCP servers.
    Config(config::Config),
    /// Resume a previous conversation.
    Resume {
        /// Conversation file to resume. If omitted, shows a conversation picker.
        file: Option<String>,
    },
}

/// Connect to daemon, failing if not reachable.
async fn connect(use_tcp: bool) -> Result<Runner> {
    if use_tcp {
        connect_tcp().await
    } else {
        connect_default().await
    }
}

/// Connect using the platform default transport: UDS on Unix, TCP on Windows.
pub(crate) async fn connect_default() -> Result<Runner> {
    #[cfg(unix)]
    {
        let socket_path = &*wcore::paths::SOCKET_PATH;
        Runner::connect(socket_path).await.with_context(|| {
            format!(
                "daemon not running — start with: crabtalk start\n  (tried {})",
                socket_path.display()
            )
        })
    }
    #[cfg(not(unix))]
    {
        connect_tcp().await
    }
}

/// Connect to crabtalk daemon via TCP, reading the port from the port file.
pub(crate) async fn connect_tcp() -> Result<Runner> {
    let tcp_port_file = &*wcore::paths::TCP_PORT_FILE;
    let port_str = std::fs::read_to_string(tcp_port_file).with_context(|| {
        format!(
            "daemon not running — start with: crabtalk start\n  (no port file at {})",
            tcp_port_file.display()
        )
    })?;
    let port: u16 = port_str
        .trim()
        .parse()
        .with_context(|| format!("invalid port in {}", tcp_port_file.display()))?;
    Runner::connect_tcp(port)
        .await
        .with_context(|| format!("failed to connect to crabtalk daemon via TCP on port {port}"))
}
