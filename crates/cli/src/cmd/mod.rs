//! CLI argument parsing and command dispatch.

use crate::repl::runner::Runner;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use futures_util::StreamExt;
use std::ffi::OsString;

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
    /// Run the daemon in the foreground.
    #[arg(long)]
    pub foreground: bool,
    /// Stop the daemon service.
    #[arg(long)]
    pub stop: bool,
    /// Stream daemon events.
    #[arg(long)]
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
        if self.foreground {
            return foreground::start().await;
        }
        if self.stop {
            return service::uninstall();
        }
        if self.events {
            let mut runner = connect_default_or_tcp(self.tcp).await?;
            let stream = runner.subscribe_events();
            tokio::pin!(stream);
            while let Some(Ok(event)) = stream.next().await {
                println!(
                    "[{}] {} (session {})",
                    event.agent, event.content, event.session
                );
            }
            return Ok(());
        }

        match self.command {
            None => {
                let runner = connect_or_start(self.tcp).await?;
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
                        let mut repl = crate::repl::ChatRepl::new(runner, "crab".into())?;
                        repl.resume(path).await
                    } else {
                        Ok(())
                    }
                }
            }
            Some(Command::Pull { package, force }) => {
                use std::io::Write;
                use wcore::protocol::message::hub_event;
                let mut runner = connect_default_or_tcp(self.tcp).await?;
                let mut stream = std::pin::pin!(runner.install_package(&package, "", "", force));
                let mut last_was_output = false;
                while let Some(event) = stream.next().await {
                    match event? {
                        hub_event::Event::Step(s) => {
                            if last_was_output {
                                println!();
                                last_was_output = false;
                            }
                            println!("  {}", s.message);
                        }
                        hub_event::Event::SetupOutput(o) => {
                            print!("\r  {}", o.content);
                            let _ = std::io::stdout().flush();
                            last_was_output = true;
                        }
                        hub_event::Event::Warning(w) => {
                            if last_was_output {
                                println!();
                                last_was_output = false;
                            }
                            eprintln!("  warning: {}", w.message);
                        }
                        hub_event::Event::Done(d) => {
                            if last_was_output {
                                println!();
                            }
                            if !d.error.is_empty() {
                                anyhow::bail!("{}", d.error);
                            }
                        }
                    }
                }
                println!("Done: {package}");
                Ok(())
            }
            Some(Command::Rm { package }) => {
                let mut runner = connect_default_or_tcp(self.tcp).await?;
                let mut stream = std::pin::pin!(runner.uninstall_package(&package));
                while let Some(event) = stream.next().await {
                    match event? {
                        wcore::protocol::message::hub_event::Event::Step(s) => {
                            println!("  {}", s.message);
                        }
                        wcore::protocol::message::hub_event::Event::Warning(w) => {
                            eprintln!("  warning: {}", w.message);
                        }
                        wcore::protocol::message::hub_event::Event::Done(d) => {
                            if !d.error.is_empty() {
                                anyhow::bail!("{}", d.error);
                            }
                        }
                        _ => {}
                    }
                }
                println!("Done: {package}");
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
            Some(Command::Logs { tail_args }) => crabtalk_command::view_logs("daemon", &tail_args),
            Some(Command::Reload) => {
                let mut runner = connect_default_or_tcp(self.tcp).await?;
                runner.reload().await?;
                println!("daemon reloaded");
                Ok(())
            }
            Some(Command::External(args)) => external::run(args),
        }
    }
}

/// Top-level subcommands.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Install a hub package.
    Pull {
        /// Package identifier in `scope/name` format.
        package: String,
        /// Overwrite if already installed.
        #[arg(long)]
        force: bool,
    },
    /// Uninstall a hub package.
    Rm {
        /// Package identifier in `scope/name` format.
        package: String,
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
    /// Hot-reload daemon config.
    Reload,
    /// Resume a previous chat session.
    Resume {
        /// Session file to resume. If omitted, shows a session picker.
        file: Option<String>,
    },
    /// Forward to an external `crabtalk-{name}` binary.
    #[command(external_subcommand)]
    External(Vec<OsString>),
}

/// Connect to daemon, auto-starting it if not reachable.
async fn connect_or_start(use_tcp: bool) -> Result<Runner> {
    match connect_default_or_tcp(use_tcp).await {
        Ok(runner) => Ok(runner),
        Err(_) => {
            // Daemon not running — start it.
            daemon::config::scaffold_config_dir(&wcore::paths::CONFIG_DIR)?;
            let config_path = wcore::paths::CONFIG_DIR.join(wcore::paths::CONFIG_FILE);
            let config = daemon::DaemonConfig::load(&config_path)?;
            if config.provider.is_empty() {
                attach::setup_provider(&config_path)?;
            }
            service::install(1, false)?;
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
