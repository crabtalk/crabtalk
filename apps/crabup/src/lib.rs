//! crabup — package manager for the Crabtalk ecosystem.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::registry::Entry;

pub mod cargo;
pub mod list;
pub mod package;
pub mod ps;
pub mod registry;
pub mod service;

#[derive(Parser, Debug)]
#[command(name = "crabup", about = "Crabtalk package and service manager")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Install a crabtalk binary from crates.io.
    Pull {
        /// Short name (daemon, tui, telegram, …) or crate name.
        name: String,
        /// Pin to a specific version.
        #[arg(long)]
        version: Option<String>,
        /// Comma-separated cargo features to enable.
        #[arg(long, value_delimiter = ',')]
        features: Vec<String>,
        /// Disable default cargo features.
        #[arg(long)]
        no_default_features: bool,
    },
    /// Uninstall a crabtalk binary.
    Rm {
        /// Short name or crate name.
        name: String,
    },
    /// Bump every installed crabtalk-* crate to the latest version.
    Update,
    /// List installed crabtalk-* crates.
    List,
    /// List running crabtalk services.
    Ps,

    /// Daemon service commands.
    Daemon {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Telegram gateway service commands.
    Telegram {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// WeChat gateway service commands.
    Wechat {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Search service commands.
    Search {
        #[command(subcommand)]
        action: ServiceAction,
    },

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

#[derive(Subcommand, Debug)]
pub enum ServiceAction {
    /// Install and start the service.
    Start {
        /// Re-install even if already running.
        #[arg(short, long)]
        force: bool,
    },
    /// Stop and uninstall the service.
    Stop,
    /// Restart the service.
    Restart,
    /// View service logs.
    Logs {
        /// Arguments passed through to `tail` (e.g. `-f`, `-n 100`).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        tail_args: Vec<String>,
    },
}

impl ServiceAction {
    fn run(self, entry: &Entry) -> Result<()> {
        match self {
            Self::Start { force } => entry.start(force),
            Self::Stop => entry.stop(),
            Self::Restart => entry.restart(),
            Self::Logs { tail_args } => entry.logs(&tail_args),
        }
    }
}

impl Cli {
    pub async fn run(self) -> Result<()> {
        match self.command {
            Command::Pull {
                name,
                version,
                features,
                no_default_features,
            } => cargo::install(
                Entry::resolve(&name),
                cargo::InstallOpts {
                    version: version.as_deref(),
                    features: &features,
                    no_default_features,
                },
            ),
            Command::Rm { name } => cargo::uninstall(Entry::resolve(&name)),
            Command::Update => {
                for krate in list::installed()? {
                    println!("==> {krate}");
                    cargo::install(&krate, cargo::InstallOpts::default())?;
                }
                Ok(())
            }
            Command::List => {
                for krate in list::installed()? {
                    println!("{krate}");
                }
                Ok(())
            }
            Command::Ps => ps::run(),
            Command::Daemon { action } => action.run(&registry::DAEMON),
            Command::Telegram { action } => action.run(&registry::TELEGRAM),
            Command::Wechat { action } => action.run(&registry::WECHAT),
            Command::Search { action } => action.run(&registry::SEARCH),
            Command::Add {
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
            Command::Remove { name } => {
                package::uninstall(&name, |msg| println!("  {msg}")).await?;
                println!("Done: {name}");
                Ok(())
            }
        }
    }
}
