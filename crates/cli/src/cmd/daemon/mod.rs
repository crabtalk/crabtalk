//! `walrus daemon` — daemon lifecycle management.

use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::Path;

mod logs;
mod service;
mod start;

/// Manage the walrus daemon.
#[derive(Args, Debug)]
pub struct Daemon {
    #[command(subcommand)]
    pub command: DaemonCommand,
}

#[derive(Subcommand, Debug)]
pub enum DaemonCommand {
    /// Start the daemon in the foreground.
    Start {
        /// Increase log verbosity (-v = info, -vv = debug, -vvv = trace).
        #[arg(short, long, action = clap::ArgAction::Count)]
        verbose: u8,
    },
    /// Trigger a hot-reload of the running daemon.
    Reload,
    /// Restart the daemon (requires system service).
    Restart,
    /// View daemon or service logs (delegates to `tail`).
    ///
    /// Extra flags (e.g. `-f`, `-n 100`, `-t`) are passed through to `tail`.
    /// Defaults to `-n 50` if no flags are given.
    Logs {
        /// Service name (e.g. "telegram"). Omit for the daemon log.
        #[arg(long)]
        service: Option<String>,
        /// Arguments passed through to `tail` (e.g. `-f`, `-n 100`).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        tail_args: Vec<String>,
    },
    /// Install walrus as a system service (launchd/systemd).
    Install,
    /// Uninstall the walrus system service.
    Uninstall,
}

impl Daemon {
    pub async fn run(self, socket_path: &Path) -> Result<()> {
        match self.command {
            DaemonCommand::Start { .. } => start::start().await,
            DaemonCommand::Reload => reload(socket_path).await,
            DaemonCommand::Restart => service::restart(),
            DaemonCommand::Logs { service, tail_args } => {
                let args = if tail_args.is_empty() {
                    vec!["-n".to_owned(), "50".to_owned()]
                } else {
                    tail_args
                };
                logs::logs(service.as_deref(), &args)
            }
            DaemonCommand::Install => service::install(),
            DaemonCommand::Uninstall => service::uninstall(),
        }
    }
}

async fn reload(socket_path: &Path) -> Result<()> {
    let mut runner = crate::repl::runner::Runner::connect(socket_path).await?;
    runner.reload().await?;
    println!("daemon reloaded");
    Ok(())
}
