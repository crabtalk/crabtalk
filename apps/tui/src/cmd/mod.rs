//! CLI argument parsing and command dispatch.

use crate::repl::runner::Runner;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use futures_util::StreamExt;
use std::{collections::HashMap, ffi::OsString};

pub mod attach;
pub mod config;
pub mod console;
pub mod external;
mod foreground;
mod service;

/// Crabtalk — AI agent platform.
#[derive(Parser, Debug)]
#[command(name = "crabtalk", about = "Crabtalk — AI agent platform")]
pub struct Cli {
    /// Start the daemon service without entering chat.
    #[arg(long, group = "daemon_op")]
    pub start: bool,
    /// Stop and restart the daemon service.
    #[arg(long, group = "daemon_op")]
    pub restart: bool,
    /// Run the daemon in the foreground.
    #[arg(long, group = "daemon_op")]
    pub foreground: bool,
    /// Stop the daemon service.
    #[arg(long, group = "daemon_op")]
    pub stop: bool,
    /// Hot-reload daemon config.
    #[arg(long, group = "daemon_op")]
    pub reload: bool,
    /// Stream daemon events.
    #[arg(long, group = "daemon_op")]
    pub events: bool,
    /// Increase log verbosity (-v = info, -vv = debug, -vvv = trace).
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
    /// Connect via TCP instead of Unix domain socket.
    #[arg(long)]
    pub tcp: bool,
    /// Agent to use.
    #[arg(long, default_value = "crab")]
    pub agent: String,
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Option<Command>,
}

impl Cli {
    /// Build a `RUST_LOG`-style filter string from the `-v` count.
    pub fn log_filter(&self) -> Option<&'static str> {
        if self.foreground && self.verbose > 0 {
            Some(match self.verbose {
                1 => "crabtalk=info",
                2 => "crabtalk=debug",
                _ => "crabtalk=trace",
            })
        } else if matches!(self.command, Some(Command::Pull { .. })) {
            Some("crabtalk=info")
        } else {
            None
        }
    }

    /// Parse and dispatch the CLI command.
    pub async fn run(self) -> Result<()> {
        // Flags take priority over subcommands.
        if self.start || self.restart {
            if self.restart {
                let _ = service::uninstall();
            }
            node::storage::scaffold_config_dir(&wcore::paths::CONFIG_DIR)?;
            let config_path = wcore::paths::CONFIG_DIR.join(wcore::paths::CONFIG_FILE);
            let config = node::NodeConfig::load(&config_path)?;
            if config.provider.is_empty() {
                attach::setup_provider(&config_path)?;
            }
            return service::install(self.verbose.max(1), self.restart);
        }
        if self.foreground {
            return foreground::start().await;
        }
        if self.stop {
            return service::uninstall();
        }
        if self.reload {
            let mut runner = connect_default_or_tcp(self.tcp).await?;
            runner.reload().await?;
            println!("daemon reloaded");
            return Ok(());
        }
        if self.events {
            use wcore::protocol::message::AgentEventKind;

            let mut runner = connect_default_or_tcp(self.tcp).await?;
            let stream = runner.subscribe_events();
            tokio::pin!(stream);
            // Buffer text/thinking deltas per (agent, sender) until the
            // matching end event arrives, then flush as one truncated line.
            // Boundary events from the daemon make this trivial — no more
            // inference from "next non-delta event." Note: a dropped End
            // (e.g. agent crash) leaves an entry in the map for the
            // process lifetime; acceptable for a debug CLI.
            let mut buffers: HashMap<(String, String), String> = HashMap::new();
            while let Some(Ok(event)) = stream.next().await {
                let key = (event.agent.clone(), event.sender.clone());
                match AgentEventKind::try_from(event.kind) {
                    Ok(AgentEventKind::TextStart | AgentEventKind::ThinkingStart) => {
                        buffers.insert(key, String::new());
                    }
                    Ok(AgentEventKind::TextDelta | AgentEventKind::ThinkingDelta) => {
                        if let Some(buf) = buffers.get_mut(&key) {
                            buf.push_str(&event.content);
                        }
                    }
                    Ok(end @ (AgentEventKind::TextEnd | AgentEventKind::ThinkingEnd)) => {
                        if let Some(text) = buffers.remove(&key) {
                            let label = if end == AgentEventKind::ThinkingEnd {
                                "thinking"
                            } else {
                                "text"
                            };
                            let trimmed = truncate_for_display(&text, 80);
                            println!("[{}] {label}: {trimmed}", event.agent);
                        }
                    }
                    _ => {
                        println!(
                            "[{}] {} (sender {})",
                            event.agent, event.content, event.sender
                        );
                    }
                }
            }
            return Ok(());
        }

        match self.command {
            None => {
                let runner = connect_or_start(self.tcp, self.verbose.max(1)).await?;
                let mut repl = crate::repl::ChatRepl::new(runner, self.agent)?;
                repl.run().await
            }
            Some(Command::Resume { file }) => {
                let runner = connect_default_or_tcp(self.tcp).await?;
                if let Some(path) = file {
                    let mut repl = crate::repl::ChatRepl::new(runner, self.agent)?;
                    repl.resume(std::path::PathBuf::from(path)).await
                } else {
                    let cmd = console::Console;
                    let selected = cmd.run(runner).await?;
                    if let Some(path) = selected {
                        let runner = connect_default_or_tcp(self.tcp).await?;
                        let mut repl = crate::repl::ChatRepl::new(runner, self.agent)?;
                        repl.resume(path).await
                    } else {
                        Ok(())
                    }
                }
            }
            Some(Command::Pull { plugin, force }) => {
                use std::io::Write;
                use wcore::protocol::message::plugin_event;
                let mut runner = connect_default_or_tcp(self.tcp).await?;
                let mut stream = std::pin::pin!(runner.install_plugin(&plugin, "", "", force));
                let mut last_was_output = false;
                while let Some(event) = stream.next().await {
                    match event? {
                        plugin_event::Event::Step(s) => {
                            if last_was_output {
                                println!();
                                last_was_output = false;
                            }
                            println!("  {}", s.message);
                        }
                        plugin_event::Event::SetupOutput(o) => {
                            print!("\r  {}", o.content);
                            let _ = std::io::stdout().flush();
                            last_was_output = true;
                        }
                        plugin_event::Event::Warning(w) => {
                            if last_was_output {
                                println!();
                                last_was_output = false;
                            }
                            eprintln!("  warning: {}", w.message);
                        }
                        plugin_event::Event::Done(d) => {
                            if last_was_output {
                                println!();
                            }
                            if !d.error.is_empty() {
                                anyhow::bail!("{}", d.error);
                            }
                        }
                    }
                }
                println!("Done: {plugin}");
                Ok(())
            }
            Some(Command::Rm { plugin }) => {
                let mut runner = connect_default_or_tcp(self.tcp).await?;
                let mut stream = std::pin::pin!(runner.uninstall_plugin(&plugin));
                while let Some(event) = stream.next().await {
                    match event? {
                        wcore::protocol::message::plugin_event::Event::Step(s) => {
                            println!("  {}", s.message);
                        }
                        wcore::protocol::message::plugin_event::Event::Warning(w) => {
                            eprintln!("  warning: {}", w.message);
                        }
                        wcore::protocol::message::plugin_event::Event::Done(d) => {
                            if !d.error.is_empty() {
                                anyhow::bail!("{}", d.error);
                            }
                        }
                        _ => {}
                    }
                }
                println!("Done: {plugin}");
                Ok(())
            }
            Some(Command::Config(cmd)) => cmd.run().await,
            Some(Command::Ps) => {
                let run_dir = &*wcore::paths::RUN_DIR;
                let mut found = false;
                if let Ok(entries) = std::fs::read_dir(run_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().and_then(|e| e.to_str()) != Some("port") {
                            continue;
                        }
                        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                            continue;
                        };
                        if name == "crabtalk" {
                            continue;
                        }
                        if let Ok(contents) = std::fs::read_to_string(&path)
                            && let Ok(port) = contents.trim().parse::<u16>()
                        {
                            let alive = std::net::TcpStream::connect(("127.0.0.1", port)).is_ok();
                            let status = if alive { "running" } else { "stale" };
                            println!("{name}\t:{port}\t{status}");
                            found = true;
                        }
                    }
                }
                if !found {
                    println!("no services running");
                }
                Ok(())
            }
            Some(Command::Logs { tail_args }) => command::view_logs("daemon", &tail_args),
            Some(Command::External(args)) => external::run(args),
        }
    }
}

/// Top-level subcommands.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Install a plugin.
    Pull {
        /// Plugin name.
        plugin: String,
        /// Overwrite if already installed.
        #[arg(long)]
        force: bool,
    },
    /// Uninstall a plugin.
    Rm {
        /// Plugin name.
        plugin: String,
    },
    /// Configure providers, models, and MCP servers.
    Config(config::Config),
    /// List running services.
    Ps,
    /// View daemon logs.
    Logs {
        /// Arguments passed through to `tail` (e.g. `-f`, `-n 100`).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        tail_args: Vec<String>,
    },
    /// Resume a previous conversation.
    Resume {
        /// Conversation file to resume. If omitted, shows a conversation picker.
        file: Option<String>,
    },
    /// Forward to an external `crabtalk-{name}` binary.
    #[command(external_subcommand)]
    External(Vec<OsString>),
}

/// Connect to daemon, auto-starting it if not reachable.
async fn connect_or_start(use_tcp: bool, verbose: u8) -> Result<Runner> {
    match connect_default_or_tcp(use_tcp).await {
        Ok(runner) => Ok(runner),
        Err(e) => {
            tracing::debug!("daemon not reachable, starting: {e}");
            node::storage::scaffold_config_dir(&wcore::paths::CONFIG_DIR)?;
            let config_path = wcore::paths::CONFIG_DIR.join(wcore::paths::CONFIG_FILE);
            let config = node::NodeConfig::load(&config_path)?;
            if config.provider.is_empty() {
                attach::setup_provider(&config_path)?;
            }
            service::install(verbose, false)?;
            // Wait for daemon to be reachable.
            for _ in 0..20 {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                if let Ok(runner) = connect_default_or_tcp(use_tcp).await {
                    return Ok(runner);
                }
            }
            anyhow::bail!("daemon started but not reachable after 5s")
        }
    }
}

/// Connect with the platform default transport, or TCP if explicitly requested.
async fn connect_default_or_tcp(use_tcp: bool) -> Result<Runner> {
    if use_tcp {
        connect_tcp().await
    } else {
        connect_default().await
    }
}

/// Connect using the platform default transport: UDS on Unix, TCP on Windows.
pub(crate) async fn connect_default() -> Result<Runner> {
    #[cfg(unix)]
    {
        let socket_path = &*wcore::paths::SOCKET_PATH;
        Runner::connect(socket_path).await.with_context(|| {
            format!(
                "failed to connect to crabtalk daemon at {}",
                socket_path.display()
            )
        })
    }
    #[cfg(not(unix))]
    {
        connect_tcp().await
    }
}

/// Connect to crabtalk daemon via TCP, reading the port from the port file.
pub(crate) async fn connect_tcp() -> Result<Runner> {
    let tcp_port_file = &*wcore::paths::TCP_PORT_FILE;
    let port_str = std::fs::read_to_string(tcp_port_file).with_context(|| {
        format!(
            "failed to read TCP port file at {}",
            tcp_port_file.display()
        )
    })?;
    let port: u16 = port_str
        .trim()
        .parse()
        .with_context(|| format!("invalid port in {}", tcp_port_file.display()))?;
    Runner::connect_tcp(port)
        .await
        .with_context(|| format!("failed to connect to crabtalk daemon via TCP on port {port}"))
}

/// Truncate a string to at most `max` bytes, snapping back to a UTF-8
/// char boundary, and append `...` if anything was dropped.
fn truncate_for_display(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max.saturating_sub(3);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}
