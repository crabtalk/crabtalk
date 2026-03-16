//! Hub package management command.

use crate::repl::runner::Runner;
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use daemon::ext::hub::manifest::Manifest;
use dialoguer::{Input, theme::ColorfulTheme};
use futures_util::StreamExt;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, value};
use wcore::paths::CONFIG_DIR;
use wcore::protocol::message::{HubAction, download_event};

/// Manage hub packages.
#[derive(Args, Debug)]
pub struct Hub {
    /// Hub subcommand.
    #[command(subcommand)]
    pub command: HubCommand,
}

/// Hub subcommands.
#[derive(Subcommand, Debug)]
pub enum HubCommand {
    /// Install a hub package.
    Install(HubPackage),
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

/// Package argument shared by install and uninstall.
#[derive(Args, Debug)]
pub struct HubPackage {
    /// Package identifier in `scope/name` format.
    pub package: String,

    /// Install only specific skills by name.
    #[arg(long)]
    pub skill: Vec<String>,

    /// Install only specific MCP servers by name.
    #[arg(long)]
    pub mcp: Vec<String>,

    /// Install only specific services by name.
    #[arg(long)]
    pub service: Vec<String>,

    /// Install only specific agents by name.
    #[arg(long)]
    pub agent: Vec<String>,
}

impl HubPackage {
    /// Build protocol-level filter strings from the CLI flags.
    fn filters(&self) -> Vec<String> {
        let mut out = Vec::new();
        for s in &self.skill {
            out.push(format!("skill:{s}"));
        }
        for s in &self.mcp {
            out.push(format!("mcp:{s}"));
        }
        for s in &self.service {
            out.push(format!("service:{s}"));
        }
        for s in &self.agent {
            out.push(format!("agent:{s}"));
        }
        out
    }
}

impl Hub {
    /// Run the hub command.
    pub async fn run(self, runner: &mut Runner) -> Result<()> {
        if let HubCommand::Test(t) = self.command {
            return test_manifest(&t.path);
        }

        let (package, action, filters) = match self.command {
            HubCommand::Install(p) => {
                let filters = p.filters();
                (p.package, HubAction::Install, filters)
            }
            HubCommand::Uninstall(p) => {
                let filters = p.filters();
                (p.package, HubAction::Uninstall, filters)
            }
            HubCommand::Test(_) => unreachable!(),
        };
        let completed = {
            let stream = runner.hub_stream(&package, action, filters);
            futures_util::pin_mut!(stream);
            let mut completed = false;
            while let Some(result) = stream.next().await {
                match result? {
                    download_event::Event::Created(c) => {
                        println!("Starting hub operation for {}...", c.label);
                    }
                    download_event::Event::Step(s) => println!("  {}", s.message),
                    download_event::Event::Completed(_) => {
                        println!("Done: {package}");
                        completed = true;
                    }
                    download_event::Event::Failed(f) => {
                        anyhow::bail!("hub operation failed: {}", f.error);
                    }
                    _ => {}
                }
            }
            completed
        };

        if completed && action == HubAction::Install {
            let config_path = CONFIG_DIR.join("walrus.toml");
            if config_path.exists() {
                let changed = prompt_empty_env_vars(&config_path)?;
                if changed {
                    let _ = runner.reload().await;
                    println!("Daemon reloaded.");
                }
                println!("\nRun `walrus auth` to reconfigure these values later.");
            }
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

/// Scan `[mcps.*]` and `[services.*]` in walrus.toml for empty env values,
/// prompt the user for each one, and write non-empty responses back.
/// Returns `true` if any values were filled.
fn prompt_empty_env_vars(config_path: &Path) -> Result<bool> {
    let content = std::fs::read_to_string(config_path)?;
    let mut doc: DocumentMut = content.parse()?;
    let theme = ColorfulTheme::default();
    let mut changed = false;

    for (section, label) in [("mcps", "MCP"), ("services", "Service")] {
        let Some(table) = doc.get_mut(section).and_then(|v| v.as_table_mut()) else {
            continue;
        };

        let names: Vec<String> = table.iter().map(|(k, _)| k.to_string()).collect();
        for name in &names {
            let Some(entry) = table.get_mut(name).and_then(|v| v.as_table_mut()) else {
                continue;
            };
            let Some(env) = entry.get_mut("env").and_then(|v| v.as_table_mut()) else {
                continue;
            };

            let empty_keys: Vec<String> = env
                .iter()
                .filter(|(_, v)| v.as_str().is_some_and(|s| s.is_empty()))
                .map(|(k, _)| k.to_string())
                .collect();

            if empty_keys.is_empty() {
                continue;
            }

            println!("\nConfigure {label} \"{name}\":");
            for key in &empty_keys {
                let val: String = Input::with_theme(&theme)
                    .with_prompt(format!("  {key}"))
                    .allow_empty(true)
                    .interact_text()?;
                if !val.is_empty() {
                    env.insert(key, value(&val));
                    changed = true;
                }
            }
        }
    }

    if changed {
        std::fs::write(config_path, doc.to_string())?;
    }

    Ok(changed)
}
