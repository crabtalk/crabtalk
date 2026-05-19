//! CLI argument parsing and command dispatch.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use sdk::{ConnectionInfo, Transport};
use std::path::PathBuf;

pub mod agent;
pub mod auth;
pub mod console;
pub mod mcp;
pub mod package;
pub mod reload;

/// Crabtalk CLI — interactive agent client.
#[derive(Parser, Debug)]
#[command(
    name = "crabtalk",
    about = "Crabtalk CLI — TUI client and management commands"
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

    /// Cloud authentication.
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },

    /// Manage crabtalk packages (skills + MCPs).
    Pkg {
        #[command(subcommand)]
        action: PkgAction,
    },

    /// Hot-reload daemon configuration.
    Reload,
}

#[derive(Subcommand, Debug)]
pub enum AuthAction {
    /// Log in to CrabTalk cloud (opens browser for Google OAuth).
    Login,
    /// Log out — remove gateway credentials from config.
    Logout,
}

#[derive(Subcommand, Debug)]
pub enum PkgAction {
    /// Install a crabtalk package.
    Add {
        /// Package short name.
        name: String,
        /// Pin to a specific branch of the package's source repo.
        #[arg(long)]
        branch: Option<String>,
        /// Local path to a package directory (skips registry sync).
        #[arg(long)]
        path: Option<PathBuf>,
        /// Re-install if already present.
        #[arg(short, long)]
        force: bool,
    },
    /// Uninstall a crabtalk package.
    Remove {
        /// Package short name.
        name: String,
    },
}

impl Cli {
    pub async fn run(self) -> Result<()> {
        match self.command {
            None => {
                let (transport, conn_info) = connect(self.tcp).await?;
                let mut repl = crate::repl::ChatRepl::new(transport, conn_info, self.agent)?;
                repl.run().await
            }
            Some(Command::Resume { file }) => {
                let (transport, conn_info) = connect(self.tcp).await?;
                if let Some(path) = file {
                    let mut repl = crate::repl::ChatRepl::new(transport, conn_info, self.agent)?;
                    repl.resume(std::path::PathBuf::from(path)).await
                } else {
                    let cmd = console::Console;
                    let selected = cmd.run(transport, conn_info.clone()).await?;
                    if let Some(path) = selected {
                        let (transport, conn_info) = connect(self.tcp).await?;
                        let mut repl =
                            crate::repl::ChatRepl::new(transport, conn_info, self.agent)?;
                        repl.resume(path).await
                    } else {
                        Ok(())
                    }
                }
            }
            Some(Command::Agent(cmd)) => cmd.run(self.tcp).await,
            Some(Command::Mcp(cmd)) => cmd.run(self.tcp).await,
            Some(Command::Auth { action }) => match action {
                AuthAction::Login => auth::login().await,
                AuthAction::Logout => auth::logout(),
            },
            Some(Command::Pkg { action }) => match action {
                PkgAction::Add {
                    name,
                    branch,
                    path,
                    force,
                } => {
                    package::install(
                        &name,
                        branch.as_deref(),
                        path.as_deref(),
                        force,
                        |msg| println!("  {msg}"),
                        |msg| println!("  {msg}"),
                    )
                    .await?;
                    println!("Done: {name}");
                    Ok(())
                }
                PkgAction::Remove { name } => {
                    package::uninstall(&name, |msg| println!("  {msg}")).await?;
                    println!("Done: {name}");
                    Ok(())
                }
            },
            Some(Command::Reload) => reload::run(self.tcp).await,
        }
    }
}

async fn connect(use_tcp: bool) -> Result<(Transport, ConnectionInfo)> {
    if use_tcp {
        connect_tcp().await
    } else {
        connect_default().await
    }
}

pub(crate) async fn connect_default() -> Result<(Transport, ConnectionInfo)> {
    #[cfg(unix)]
    {
        let socket_path = &*wcore::paths::SOCKET_PATH;
        let info = ConnectionInfo::Uds(socket_path.to_path_buf());
        let transport = sdk::connect_from(&info).await.with_context(|| {
            format!(
                "daemon not running — start with: crabup daemon start\n  (tried {})",
                socket_path.display()
            )
        })?;
        Ok((transport, info))
    }
    #[cfg(not(unix))]
    {
        connect_tcp().await
    }
}

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

pub(crate) async fn connect_tcp() -> Result<(Transport, ConnectionInfo)> {
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
    let info = ConnectionInfo::Tcp(port);
    let transport = sdk::connect_from(&info)
        .await
        .with_context(|| format!("failed to connect to crabtalk daemon via TCP on port {port}"))?;
    Ok((transport, info))
}
