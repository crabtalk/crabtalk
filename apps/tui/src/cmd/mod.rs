//! CLI argument parsing and command dispatch.

use crate::repl::runner::Runner;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
#[cfg(feature = "daemon")]
use std::ffi::OsString;

pub mod agent;
pub mod config;
pub mod console;
pub mod mcp;

/// Crabtalk TUI — interactive agent client.
#[derive(Parser, Debug)]
#[command(
    name = "crabtalk-tui",
    about = "Crabtalk TUI — interactive agent client"
)]
pub struct Cli {
    /// Run the daemon in the foreground.
    #[cfg(feature = "daemon")]
    #[arg(long)]
    pub foreground: bool,
    /// Start the daemon service without entering chat.
    #[cfg(feature = "daemon")]
    #[arg(long, group = "daemon_op")]
    pub start: bool,
    /// Stop and restart the daemon service.
    #[cfg(feature = "daemon")]
    #[arg(long, group = "daemon_op")]
    pub restart: bool,
    /// Stop the daemon service.
    #[cfg(feature = "daemon")]
    #[arg(long, group = "daemon_op")]
    pub stop: bool,
    /// Hot-reload daemon config.
    #[cfg(feature = "daemon")]
    #[arg(long, group = "daemon_op")]
    pub reload: bool,
    /// Stream daemon events.
    #[cfg(feature = "daemon")]
    #[arg(long, group = "daemon_op")]
    pub events: bool,
    /// Increase log verbosity (-v = info, -vv = debug, -vvv = trace).
    #[cfg(feature = "daemon")]
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
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
    /// Configure providers, models, and MCP servers.
    Config(config::Config),
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
    /// List running services.
    #[cfg(feature = "daemon")]
    Ps,
    /// View daemon logs.
    #[cfg(feature = "daemon")]
    Logs {
        /// Arguments passed through to `tail` (e.g. `-f`, `-n 100`).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        tail_args: Vec<String>,
    },
    /// Forward to an external `crabtalk-{name}` binary.
    #[cfg(feature = "daemon")]
    #[command(external_subcommand)]
    External(Vec<OsString>),
}

impl Cli {
    /// Parse and dispatch the CLI command.
    pub async fn run(self) -> Result<()> {
        // Daemon flags take priority.
        #[cfg(feature = "daemon")]
        {
            if self.foreground {
                return crabtalkd::foreground::start().await;
            }
            if self.start || self.restart {
                if self.restart {
                    let _ = crabtalkd::service::uninstall();
                }
                crabtalkd::ensure_config()?;
                return crabtalkd::service::install(self.verbose.max(1), self.restart);
            }
            if self.stop {
                return crabtalkd::service::uninstall();
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
                #[cfg(feature = "daemon")]
                let runner = connect_or_start(self.tcp, self.verbose.max(1)).await?;
                #[cfg(not(feature = "daemon"))]
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
            #[cfg(feature = "daemon")]
            Some(Command::Ps) => {
                let daemon = crabtalkd::Cli {
                    foreground: false,
                    verbose: 0,
                    tcp: self.tcp,
                    command: Some(crabtalkd::Command::Ps),
                };
                daemon.run().await
            }
            #[cfg(feature = "daemon")]
            Some(Command::Logs { tail_args }) => {
                let daemon = crabtalkd::Cli {
                    foreground: false,
                    verbose: 0,
                    tcp: self.tcp,
                    command: Some(crabtalkd::Command::Logs { tail_args }),
                };
                daemon.run().await
            }
            #[cfg(feature = "daemon")]
            Some(Command::External(args)) => {
                let daemon = crabtalkd::Cli {
                    foreground: false,
                    verbose: 0,
                    tcp: self.tcp,
                    command: Some(crabtalkd::Command::External(args)),
                };
                daemon.run().await
            }
        }
    }
}

/// Connect to daemon, auto-starting it if not reachable.
#[cfg(feature = "daemon")]
async fn connect_or_start(use_tcp: bool, verbose: u8) -> Result<Runner> {
    match connect(use_tcp).await {
        Ok(runner) => Ok(runner),
        Err(e) => {
            tracing::debug!("daemon not reachable, starting: {e}");
            crabtalkd::ensure_config()?;
            // We just confirmed the daemon isn't reachable. Force a clean
            // reinstall — without this, a stale plist (zombie daemon, crashed
            // before opening the socket) makes `service::install` no-op with
            // "daemon is already running" and we loop until the 5s timeout.
            crabtalkd::service::install(verbose, true)?;
            for _ in 0..20 {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                if let Ok(runner) = connect(use_tcp).await {
                    return Ok(runner);
                }
            }
            anyhow::bail!("daemon started but not reachable after 5s")
        }
    }
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
