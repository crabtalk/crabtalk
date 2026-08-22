//! The commands — everything crabup does to an install.
//!
//! Gated behind `cmd` so a crate that only needs the layout in [`crate::dirs`]
//! does not build a CLI.

use anyhow::Result;

pub mod cargo;
pub mod list;

/// What crabup manages, as `(crate, binary)` — the two differ, and the one
/// you type is the one cargo never mentions. Installed and removed
/// together: these speak one protobuf protocol to each other, so a machine
/// holding two versions of it is the failure a separate install would
/// eventually produce. `crabtalk-cli` joins the list when it exists.
pub const CRATES: &[(&str, &str)] = &[("crabtalk-agent", "crabtalkd")];

#[derive(clap::Parser, Debug)]
#[command(name = "crabup", about = "Crabtalk version manager")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand, Debug)]
pub enum Command {
    /// Install crabtalk, or move it to the latest version.
    #[command(visible_alias = "update")]
    Install {
        #[command(flatten)]
        fetch: Fetch,
    },
    /// Uninstall crabtalk.
    Uninstall,
    /// Show what is installed.
    List,
}

/// Which build to install.
#[derive(clap::Args, Debug)]
pub struct Fetch {
    /// Pin to a specific version (e.g. 0.0.21).
    #[arg(long, conflicts_with = "nightly")]
    pub version: Option<String>,
    /// Build from the development branch instead of the release on crates.io.
    #[arg(long)]
    pub nightly: bool,
    /// Comma-separated cargo features.
    #[arg(long, value_delimiter = ',')]
    pub features: Vec<String>,
    /// Disable default cargo features.
    #[arg(long)]
    pub no_default_features: bool,
}

impl Fetch {
    /// `cargo install` is already an upgrade when a newer version exists and
    /// a no-op when it does not, so installing and updating are one act.
    fn run(self) -> Result<()> {
        for (krate, _) in CRATES.iter().copied() {
            cargo::install(
                krate,
                cargo::InstallOpts {
                    version: self.version.as_deref(),
                    features: &self.features,
                    no_default_features: self.no_default_features,
                    nightly: self.nightly,
                },
            )?;
        }
        Ok(())
    }
}

impl Cli {
    pub fn run(self) -> Result<()> {
        match self.command {
            Command::Install { fetch } => fetch.run(),
            Command::Uninstall => CRATES
                .iter()
                .try_for_each(|(krate, _)| cargo::uninstall(krate)),
            Command::List => list::run(),
        }
    }
}
