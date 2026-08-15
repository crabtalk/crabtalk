//! crabup — version manager for the Crabtalk ecosystem.

use anyhow::{Result, anyhow};

use crate::registry::{Entry, Kind};

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
    /// Install an app — daemon, cli, telegram — or any crate by name.
    Install {
        #[command(flatten)]
        fetch: Fetch,
    },
    /// Add a harness service — search.
    Add {
        #[command(flatten)]
        fetch: Fetch,
    },
    /// Uninstall an app, or any crate by name.
    Uninstall {
        #[command(flatten)]
        target: Removal,
    },
    /// Remove a harness service.
    Remove {
        #[command(flatten)]
        target: Removal,
    },
    /// Update all installed crabtalk binaries to the latest version.
    Update,
    /// List available crabtalk binaries (installed + running status).
    List,

    /// `<name> <args…>` — forward to the service binary.
    #[command(external_subcommand)]
    Service(Vec<String>),
}

/// Everything `install` and `add` share. They differ only in which
/// [`Kind`] of binary they accept — the fetch itself is identical.
#[derive(clap::Args, Debug)]
pub struct Fetch {
    /// Short name (daemon, cli, telegram, …) or crate name.
    #[arg(required = true)]
    pub names: Vec<String>,
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

/// Reject names the verb for `kind` shouldn't handle, before any of them
/// is acted on — a mixed list fails whole rather than half-applying.
/// `verb` picks the install or remove name for a `Kind`, so the same gate
/// serves `install`/`add` and `uninstall`/`remove`.
fn gate(names: &[String], kind: Kind, verb: fn(Kind) -> &'static str) -> Result<()> {
    for name in names {
        match Entry::by_short(name) {
            Some(entry) if entry.kind != kind => {
                return Err(anyhow!(
                    "{name} is a {}, not a {}: use `crabup {} {name}`",
                    entry.kind.noun(),
                    kind.noun(),
                    verb(entry.kind)
                ));
            }
            // An unknown name is an arbitrary crate. Nothing marks it as
            // harness, so it belongs to the app verb.
            None if kind == Kind::Harness => {
                return Err(anyhow!(
                    "unknown harness service: {name} — `crabup {}` takes {}; \
                     for any other crate use `crabup {} {name}`",
                    verb(Kind::Harness),
                    Entry::shorts_of(Kind::Harness).join(", "),
                    verb(Kind::App)
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

impl Fetch {
    fn run(self, kind: Kind) -> Result<()> {
        gate(&self.names, kind, Kind::install_verb)?;

        let use_source = self.source || !self.features.is_empty() || self.no_default_features;
        if use_source {
            for name in &self.names {
                let krate = Entry::resolve(name);
                cargo::install(
                    krate,
                    cargo::InstallOpts {
                        version: self.version.as_deref(),
                        features: &self.features,
                        no_default_features: self.no_default_features,
                    },
                )?;
            }
            return Ok(());
        }

        let mut entries: Vec<&Entry> = Vec::new();
        let mut cargo_names: Vec<&str> = Vec::new();
        for name in &self.names {
            match Entry::by_short(name) {
                Some(entry) => entries.push(entry),
                None => cargo_names.push(name),
            }
        }

        if !entries.is_empty() {
            match github::install(&entries, self.version.as_deref()) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!(
                        "warn: github download failed ({e:#}), falling back to cargo install"
                    );
                    for entry in &entries {
                        cargo::install(
                            entry.krate,
                            cargo::InstallOpts {
                                version: self.version.as_deref(),
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
                    version: self.version.as_deref(),
                    ..Default::default()
                },
            )?;
        }

        Ok(())
    }
}

/// The other half of [`Fetch`] — same kind gate, opposite direction.
#[derive(clap::Args, Debug)]
pub struct Removal {
    /// Short name or crate name.
    pub name: String,
}

impl Removal {
    fn run(self, kind: Kind) -> Result<()> {
        gate(std::slice::from_ref(&self.name), kind, Kind::remove_verb)?;

        if let Some(entry) = Entry::by_short(&self.name) {
            let managed = wcore::paths::BIN_DIR.join(entry.bin);
            if managed.exists() {
                std::fs::remove_file(&managed)?;
                manifest::remove(entry.short)?;
                println!("info: removed {}", managed.display());
                return Ok(());
            }
        }
        cargo::uninstall(Entry::resolve(&self.name))
    }
}

impl Cli {
    pub fn run(self) -> Result<()> {
        match self.command {
            Command::Install { fetch } => fetch.run(Kind::App),
            Command::Add { fetch } => fetch.run(Kind::Harness),
            Command::Uninstall { target } => target.run(Kind::App),
            Command::Remove { target } => target.run(Kind::Harness),
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
            "{} not installed — run `crabup {} {}` first",
            entry.bin,
            entry.kind.install_verb(),
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
