//! crabup — version manager for the Crabtalk ecosystem.

use anyhow::{Result, anyhow};

use crate::registry::Entry;

pub mod cargo;
pub mod list;
pub mod registry;

#[derive(clap::Parser, Debug)]
#[command(name = "crabup", about = "Crabtalk version manager")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand, Debug)]
pub enum Command {
    /// Install a crabtalk binary from crates.io.
    Install {
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
    Uninstall {
        /// Short name or crate name.
        name: String,
    },
    /// Bump every installed crabtalk-* crate to the latest version.
    Update,
    /// List available crabtalk binaries (installed + running status).
    List,

    /// `<name> <args…>` — forward to the service binary.
    #[command(external_subcommand)]
    Service(Vec<String>),
}

impl Cli {
    pub fn run(self) -> Result<()> {
        match self.command {
            Command::Install {
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
            Command::Uninstall { name } => cargo::uninstall(Entry::resolve(&name)),
            Command::Update => {
                for krate in list::installed()? {
                    println!("==> {krate}");
                    cargo::install(&krate, cargo::InstallOpts::default())?;
                }
                Ok(())
            }
            Command::List => list::run(),
            Command::Service(args) => forward_service(args),
        }
    }
}

fn forward_service(args: Vec<String>) -> Result<()> {
    let mut iter = args.into_iter();
    let name = iter.next().ok_or_else(|| anyhow!("missing service name"))?;
    let entry = Entry::by_short(&name).ok_or_else(|| anyhow!("unknown service: {name}"))?;
    let binary = entry.binary_path().ok_or_else(|| {
        anyhow!(
            "{} not installed — run `crabup install {}` first",
            entry.krate,
            entry.short
        )
    })?;
    let remaining: Vec<String> = iter.collect();
    let status = std::process::Command::new(&binary)
        .args(&remaining)
        .status()
        .map_err(|e| anyhow!("failed to exec {}: {e}", binary.display()))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}
