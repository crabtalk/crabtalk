//! The commands — everything crabup does to an install.
//!
//! Gated behind `cmd` so a crate that only needs the layout in [`crate::dirs`]
//! does not build a CLI, an HTTP client, and a tar decoder.

use anyhow::Result;

pub mod cargo;
pub mod github;
pub mod list;
pub mod manifest;

/// The binary crabup manages, as its crate and as its file on disk.
pub const AGENT: &str = "crabtalk-agent";

#[derive(clap::Parser, Debug)]
#[command(name = "crabup", about = "Crabtalk version manager")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand, Debug)]
pub enum Command {
    /// Install crabtalk.
    Install {
        #[command(flatten)]
        fetch: Fetch,
    },
    /// Uninstall crabtalk.
    Uninstall,
    /// Update crabtalk to the latest version.
    Update,
    /// Show what is installed.
    List,
}

/// Which build of the binary to fetch, and how.
#[derive(clap::Args, Debug)]
pub struct Fetch {
    /// Pin to a specific version (e.g. v0.0.21).
    #[arg(long)]
    pub version: Option<String>,
    /// Build from source via cargo install instead of downloading.
    #[arg(long)]
    pub source: bool,
    /// Comma-separated cargo features (implies --source).
    #[arg(long, value_delimiter = ',')]
    pub features: Vec<String>,
    /// Disable default cargo features (implies --source).
    #[arg(long)]
    pub no_default_features: bool,
}

impl Fetch {
    /// A release build unless the flags ask for something cargo has to
    /// compile, with cargo as the fallback when no release serves this
    /// platform.
    fn run(self) -> Result<()> {
        let opts = cargo::InstallOpts {
            version: self.version.as_deref(),
            features: &self.features,
            no_default_features: self.no_default_features,
        };
        if self.source || !self.features.is_empty() || self.no_default_features {
            return cargo::install(AGENT, opts);
        }

        match github::install(self.version.as_deref()) {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("warn: github download failed ({e:#}), falling back to cargo install");
                cargo::install(AGENT, opts)
            }
        }
    }
}

/// Remove the managed binary, or the cargo-installed one if that is what
/// is there.
fn uninstall() -> Result<()> {
    let managed = crate::dirs::BIN_DIR.join(AGENT);
    if managed.exists() {
        std::fs::remove_file(&managed)?;
        manifest::remove(AGENT)?;
        println!("info: removed {}", managed.display());
        return Ok(());
    }
    cargo::uninstall(AGENT)
}

fn update() -> Result<()> {
    let Some(current) = manifest::version(AGENT) else {
        println!("nothing installed via crabup");
        return Ok(());
    };

    println!("info: checking latest version...");
    let latest = github::latest_version()?;
    if current == latest {
        println!("{AGENT} {latest} is up to date");
        return Ok(());
    }

    println!("info: updating {AGENT} {current} → {latest}");
    github::install(Some(&latest))
}

impl Cli {
    pub fn run(self) -> Result<()> {
        match self.command {
            Command::Install { fetch } => fetch.run(),
            Command::Uninstall => uninstall(),
            Command::Update => update(),
            Command::List => list::run(),
        }
    }
}
