//! Hub package management command.

use crate::repl::{self, runner::Runner};
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use crabhub::manifest::Manifest;
use std::path::{Path, PathBuf};
use wcore::Setup;

/// Manage hub packages.
#[derive(Args, Debug)]
pub struct Hub {
    /// Branch of the hub repo to sync.
    #[arg(long)]
    pub branch: Option<String>,
    /// Path to a local hub repo (skip remote sync).
    #[arg(long)]
    pub path: Option<PathBuf>,
    /// Hub subcommand.
    #[command(subcommand)]
    pub command: HubCommand,
}

/// Hub subcommands.
#[derive(Subcommand, Debug)]
pub enum HubCommand {
    /// Install a hub package.
    Install(HubInstall),
    /// Uninstall a hub package.
    Uninstall(HubPackage),
    /// Test manifest parsing for all .toml files in a hub directory.
    Test(HubTest),
}

/// Arguments for the test subcommand.
#[derive(Args, Debug)]
pub struct HubTest {
    /// Path to a manifest .toml file to validate.
    pub path: PathBuf,
}

/// Install arguments.
#[derive(Args, Debug)]
pub struct HubInstall {
    /// Package identifier in `scope/name` format.
    pub package: String,
    /// Overwrite if already installed.
    #[arg(long)]
    pub force: bool,
}

/// Package argument shared by uninstall.
#[derive(Args, Debug)]
pub struct HubPackage {
    /// Package identifier in `scope/name` format.
    pub package: String,
}

impl Hub {
    /// Run the hub command.
    pub async fn run(self, runner: &mut Runner) -> Result<()> {
        if let HubCommand::Test(t) = self.command {
            return test_manifest(&t.path);
        }

        let (pkg, force, is_install) = match self.command {
            HubCommand::Install(p) => (p.package, p.force, true),
            HubCommand::Uninstall(p) => (p.package, false, false),
            HubCommand::Test(_) => unreachable!(),
        };

        let on_step = |msg: &str| println!("  {msg}");

        if is_install {
            let result = crabhub::package::install(
                &pkg,
                self.branch.as_deref(),
                self.path.as_deref(),
                force,
                on_step,
            )
            .await?;
            println!("Done: {pkg}");

            // Reload daemon to pick up new components.
            let _ = runner.reload().await;
            println!("Daemon reloaded.");

            // Check for conflicts with existing packages.
            let config_dir = &*wcore::paths::CONFIG_DIR;
            let (manifest, mut warnings) = wcore::resolve_manifests(config_dir);
            warnings.extend(wcore::check_skill_conflicts(&manifest.skill_dirs));
            for w in &warnings {
                eprintln!("  warning: {w}");
            }

            // Run prompt-type setup via inference.
            if let Some(Setup::Prompt { ref prompt }) = result.setup {
                let prompt_text = if prompt.ends_with(".md") {
                    let repo_dir = result
                        .repo_dir
                        .as_ref()
                        .context("prompt setup requires a repository but none was cloned")?;
                    let raw = std::fs::read_to_string(repo_dir.join(prompt))
                        .with_context(|| format!("failed to read setup prompt: {}", prompt))?;
                    // Replace <REPO_DIR> placeholder with the actual cached repo path.
                    raw.replace("<REPO_DIR>", &repo_dir.display().to_string())
                } else {
                    prompt.clone()
                };

                println!("Running setup…");
                let conn_info = runner.conn_info().clone();
                let stream = runner.stream(
                    wcore::paths::DEFAULT_AGENT,
                    &prompt_text,
                    result.repo_dir.as_deref(),
                );
                repl::stream_to_terminal(stream, &conn_info).await?;
                println!();
            }

            println!("Configure env vars in config.toml [env] section if needed.");
        } else {
            crabhub::package::uninstall(&pkg, on_step).await?;
            println!("Done: {pkg}");

            // Reload daemon to drop removed components.
            let _ = runner.reload().await;
            println!("Daemon reloaded.");
        }

        Ok(())
    }
}

/// Parse a single manifest .toml and report success or the parse error.
fn test_manifest(path: &Path) -> Result<()> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let manifest: Manifest =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    println!("ok  {}", manifest.package.name);
    Ok(())
}
