//! CLI argument parsing and command dispatch.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use sdk::Client;
#[cfg(feature = "daemon")]
pub mod agent;
pub mod console;
pub mod mcp;

/// Crabtalk TUI — interactive agent client.
#[derive(Parser, Debug)]
#[command(
    name = "crabtalk-tui",
    about = "Crabtalk TUI — interactive agent client"
)]
pub struct Cli {
    /// Run the daemon in the foreground (all-in-one mode).
    #[cfg(feature = "daemon")]
    #[arg(long)]
    pub foreground: bool,
    /// Hot-reload daemon config.
    #[cfg(feature = "daemon")]
    #[arg(long, group = "daemon_op")]
    pub reload: bool,
    /// Stream daemon events.
    #[cfg(feature = "daemon")]
    #[arg(long, group = "daemon_op")]
    pub events: bool,
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

/// Top-level subcommands.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Manage agents (create, list, delete, rename).
    Agent(agent::Agent),
    /// Manage MCP servers (create, list, delete).
    Mcp(mcp::Mcp),
    /// Resume a previous conversation.
    Resume {
        /// Conversation file to resume. If omitted, shows a conversation picker.
        file: Option<String>,
    },
    /// Install a plugin.
    #[cfg(feature = "daemon")]
    Pull {
        /// Plugin name.
        plugin: String,
        /// Overwrite if already installed.
        #[arg(long)]
        force: bool,
    },
    /// Uninstall a plugin.
    #[cfg(feature = "daemon")]
    Rm {
        /// Plugin name.
        plugin: String,
    },
}

impl Cli {
    /// Parse and dispatch the CLI command.
    pub async fn run(self) -> Result<()> {
        #[cfg(feature = "daemon")]
        {
            if self.foreground {
                return crabtalkd::foreground::start().await;
            }
            if self.reload {
                let daemon = crabtalkd::Cli {
                    foreground: false,
                    verbose: 0,
                    tcp: self.tcp,
                    command: Some(crabtalkd::Command::Reload),
                };
                return daemon.run().await;
            }
            if self.events {
                let daemon = crabtalkd::Cli {
                    foreground: false,
                    verbose: 0,
                    tcp: self.tcp,
                    command: Some(crabtalkd::Command::Events),
                };
                return daemon.run().await;
            }
        }

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
            Some(Command::Agent(cmd)) => cmd.run(self.tcp).await,
            Some(Command::Mcp(cmd)) => cmd.run(self.tcp).await,
            #[cfg(feature = "daemon")]
            Some(Command::Pull { plugin, force }) => {
                let daemon = crabtalkd::Cli {
                    foreground: false,
                    verbose: 0,
                    tcp: self.tcp,
                    command: Some(crabtalkd::Command::Pull { plugin, force }),
                };
                daemon.run().await
            }
            #[cfg(feature = "daemon")]
            Some(Command::Rm { plugin }) => {
                let daemon = crabtalkd::Cli {
                    foreground: false,
                    verbose: 0,
                    tcp: self.tcp,
                    command: Some(crabtalkd::Command::Rm { plugin }),
                };
                daemon.run().await
            }
        }
    }
}

/// Connect to daemon, failing with a clear message pointing at crabup if down.
async fn connect(use_tcp: bool) -> Result<Client> {
    if use_tcp {
        connect_tcp().await
    } else {
        connect_default().await
    }
}

/// Connect using the platform default transport: UDS on Unix, TCP on Windows.
pub(crate) async fn connect_default() -> Result<Client> {
    #[cfg(unix)]
    {
        let socket_path = &*wcore::paths::SOCKET_PATH;
        Client::connect_uds(socket_path).await.with_context(|| {
            format!(
                "daemon not running — start with: crabup daemon start\n  (tried {})",
                socket_path.display()
            )
        })
    }
    #[cfg(not(unix))]
    {
        connect_tcp().await
    }
}

/// Read the contents of a file path, or stdin if the path is `-`.
pub(crate) fn read_path_or_stdin(path: &std::path::Path) -> Result<String> {
    if path.as_os_str() == "-" {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
            .context("failed to read stdin")?;
        Ok(buf)
    } else {
        std::fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
    }
}

/// Connect to crabtalk daemon via TCP, reading the port from the port file.
pub(crate) async fn connect_tcp() -> Result<Client> {
    let tcp_port_file = &*wcore::paths::TCP_PORT_FILE;
    let port_str = std::fs::read_to_string(tcp_port_file).with_context(|| {
        format!(
            "daemon not running — start with: crabup daemon start\n  (no port file at {})",
            tcp_port_file.display()
        )
    })?;
    let port: u16 = port_str
        .trim()
        .parse()
        .with_context(|| format!("invalid port in {}", tcp_port_file.display()))?;
    Client::connect_tcp(port)
        .await
        .with_context(|| format!("failed to connect to crabtalk daemon via TCP on port {port}"))
}
