//! crabup — version manager for the Crabtalk ecosystem.

use anyhow::{Result, anyhow};

use crate::registry::Entry;

pub mod cargo;
pub mod github;
pub mod list;
pub mod manifest;
pub mod registry;

#[derive(clap::Parser, Debug)]
#[command(name = "crabup", about = "Crabtalk version manager")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand, Debug)]
pub enum Command {
    /// Install a crabtalk binary (downloads prebuilt from GitHub releases).
    Install {
        /// Short name (daemon, cli, telegram, …) or crate name.
        #[arg(required = true)]
        names: Vec<String>,
        /// Pin to a specific version (e.g. v0.0.21).
        #[arg(long)]
        version: Option<String>,
        /// Build from source via cargo install instead of downloading.
        #[arg(long)]
        source: bool,
        /// Comma-separated cargo features (implies --source).
        #[arg(long, value_delimiter = ',')]
        features: Vec<String>,
        /// Disable default cargo features (implies --source).
        #[arg(long)]
        no_default_features: bool,
    },
    /// Uninstall a crabtalk binary.
    Uninstall {
        /// Short name or crate name.
        name: String,
    },
    /// Update all installed crabtalk binaries to the latest version.
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
                names,
                version,
                source,
                features,
                no_default_features,
            } => {
                let use_source = source || !features.is_empty() || no_default_features;
                if use_source {
                    for name in &names {
                        let krate = Entry::resolve(name);
                        cargo::install(
                            krate,
                            cargo::InstallOpts {
                                version: version.as_deref(),
                                features: &features,
                                no_default_features,
                            },
                        )?;
                    }
                    return Ok(());
                }

                let mut entries: Vec<&Entry> = Vec::new();
                let mut cargo_names: Vec<&str> = Vec::new();
                for name in &names {
                    match Entry::by_short(name) {
                        Some(entry) => entries.push(entry),
                        None => cargo_names.push(name),
                    }
                }

                if !entries.is_empty() {
                    match github::install(&entries, version.as_deref()) {
                        Ok(()) => {}
                        Err(e) => {
                            eprintln!(
                                "warn: github download failed ({e:#}), falling back to cargo install"
                            );
                            for entry in &entries {
                                cargo::install(
                                    entry.krate,
                                    cargo::InstallOpts {
                                        version: version.as_deref(),
                                        ..Default::default()
                                    },
                                )?;
                            }
                        }
                    }
                }

                for name in cargo_names {
                    cargo::install(
                        name,
                        cargo::InstallOpts {
                            version: version.as_deref(),
                            ..Default::default()
                        },
                    )?;
                }

                Ok(())
            }
            Command::Uninstall { name } => {
                if let Some(entry) = Entry::by_short(&name) {
                    let managed = wcore::paths::BIN_DIR.join(entry.bin);
                    if managed.exists() {
                        std::fs::remove_file(&managed)?;
                        manifest::remove(entry.short)?;
                        println!("info: removed {}", managed.display());
                        return Ok(());
                    }
                }
                cargo::uninstall(Entry::resolve(&name))
            }
            Command::Update => {
                let installed = manifest::all()?;
                if installed.is_empty() {
                    println!("nothing installed via crabup");
                    return Ok(());
                }

                println!("info: checking latest version...");
                let latest = github::latest_version()?;
                println!("info: latest version: {latest}");

                let outdated: Vec<&Entry> = installed
                    .iter()
                    .filter(|(_, v)| v.as_str() != latest)
                    .filter_map(|(short, _)| Entry::by_short(short))
                    .collect();

                if outdated.is_empty() {
                    println!("everything is up to date");
                    return Ok(());
                }

                println!("info: updating {} component(s)", outdated.len());
                github::install(&outdated, Some(&latest))
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
            entry.bin,
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
