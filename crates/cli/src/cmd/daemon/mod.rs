//! `crabtalk daemon` — daemon lifecycle management.

use anyhow::Result;
use clap::{Args, Subcommand};

mod logs;
mod service;
mod start;

/// Manage the crabtalk daemon.
#[derive(Args, Debug)]
pub struct Daemon {
    /// Increase log verbosity (-v = info, -vv = debug, -vvv = trace).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,
    #[command(subcommand)]
    pub command: DaemonCommand,
}

#[derive(Subcommand, Debug)]
pub enum DaemonCommand {
    /// Run the daemon in the foreground.
    Run,
    /// Trigger a hot-reload of the running daemon.
    Reload,
    /// View daemon logs (delegates to `tail`).
    ///
    /// Extra flags (e.g. `-f`, `-n 100`, `-t`) are passed through to `tail`.
    /// Defaults to `-n 50` if no flags are given.
    Logs {
        /// Arguments passed through to `tail` (e.g. `-f`, `-n 100`).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        tail_args: Vec<String>,
    },
    /// Install and start crabtalk as a system service (launchd/systemd).
    Start {
        /// Re-install even if already installed.
        #[arg(short, long)]
        force: bool,
    },
    /// Stop and uninstall the crabtalk system service.
    Stop,
}

impl Daemon {
    #[cfg(unix)]
    pub async fn run(self, socket_path: &std::path::Path) -> Result<()> {
        match self.command {
            DaemonCommand::Run => start::start().await,
            DaemonCommand::Reload => {
                let mut runner = crate::repl::runner::Runner::connect(socket_path).await?;
                runner.reload().await?;
                println!("daemon reloaded");
                Ok(())
            }
            DaemonCommand::Logs { tail_args } => logs::logs(&tail_args),
            DaemonCommand::Start { force } => service::install(self.verbose.max(1), force),
            DaemonCommand::Stop => service::uninstall(),
        }
    }

    #[cfg(not(unix))]
    pub async fn run(self) -> Result<()> {
        match self.command {
            DaemonCommand::Run => start::start().await,
            DaemonCommand::Reload => {
                let mut runner = crate::cmd::connect_tcp().await?;
                runner.reload().await?;
                println!("daemon reloaded");
                Ok(())
            }
            DaemonCommand::Logs { tail_args } => logs::logs(&tail_args),
            DaemonCommand::Start { force } => service::install(self.verbose.max(1), force),
            DaemonCommand::Stop => service::uninstall(),
        }
    }
}
