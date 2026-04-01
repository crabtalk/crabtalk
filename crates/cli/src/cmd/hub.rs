//! Hub package management command.

use crate::repl::runner::{OutputChunk, Runner, send_reply};
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use crabhub::manifest::Manifest;
use futures_util::StreamExt;
use std::path::{Path, PathBuf};
use wcore::protocol::message::hub_event;

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
        let branch = self.branch.as_deref().unwrap_or("");
        let path = self
            .path
            .as_deref()
            .map(|p| p.to_string_lossy())
            .unwrap_or_default();

        match self.command {
            HubCommand::Test(t) => return test_manifest(&t.path),
            HubCommand::Install(p) => {
                let conn_info = runner.conn_info.clone();
                let mut setup_prompt = String::new();
                {
                    let mut stream =
                        std::pin::pin!(runner.install_package(&p.package, branch, &path, p.force));
                    while let Some(event) = stream.next().await {
                        match event? {
                            hub_event::Event::Step(s) => println!("  {}", s.message),
                            hub_event::Event::Warning(w) => eprintln!("  warning: {}", w.message),
                            hub_event::Event::Setup(s) => {
                                setup_prompt = s.prompt;
                            }
                            hub_event::Event::Done(d) => {
                                if !d.error.is_empty() {
                                    anyhow::bail!("{}", d.error);
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // Run setup prompt on the client side so the user sees full
                // LLM output and can respond to ask_user prompts.
                if !setup_prompt.is_empty() {
                    println!("  running setup…");
                    let mut stream = std::pin::pin!(runner.stream(
                        wcore::paths::DEFAULT_AGENT,
                        &setup_prompt,
                        None,
                        true,
                        None,
                        Some("hub-setup".to_string()),
                    ));
                    while let Some(chunk) = stream.next().await {
                        match chunk? {
                            OutputChunk::Text(text) => print!("{text}"),
                            OutputChunk::Thinking(text) => print!("{text}"),
                            OutputChunk::ToolStart(calls) => {
                                let names: Vec<_> = calls.iter().map(|(n, _)| n.as_str()).collect();
                                println!("  [{}]", names.join(", "));
                            }
                            OutputChunk::ToolResult(_, _) | OutputChunk::ToolDone(_) => {}
                            OutputChunk::AskUser { questions, session } => {
                                for q in &questions {
                                    println!("{}", q.header);
                                    for (i, opt) in q.options.iter().enumerate() {
                                        println!("  {}: {}", i + 1, opt.label);
                                    }
                                    print!("> ");
                                    std::io::Write::flush(&mut std::io::stdout())?;
                                    let mut reply = String::new();
                                    std::io::stdin().read_line(&mut reply)?;
                                    send_reply(&conn_info, session, reply.trim().to_string())
                                        .await?;
                                }
                            }
                        }
                    }
                    println!();
                }

                println!("Done: {}", p.package);
            }
            HubCommand::Uninstall(p) => {
                let mut stream = std::pin::pin!(runner.uninstall_package(&p.package));
                while let Some(event) = stream.next().await {
                    match event? {
                        hub_event::Event::Step(s) => println!("  {}", s.message),
                        hub_event::Event::Warning(w) => eprintln!("  warning: {}", w.message),
                        hub_event::Event::Done(d) => {
                            if !d.error.is_empty() {
                                anyhow::bail!("{}", d.error);
                            }
                        }
                        _ => {}
                    }
                }
                println!("Done: {}", p.package);
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
