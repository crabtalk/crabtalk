//! CLI argument parsing and command dispatch.

use crate::repl::runner::Runner;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::ffi::OsString;

pub mod attach;
pub mod auth;
pub mod console;
pub mod daemon;
pub mod external;
pub mod hub;

/// Crabtalk CLI client — connects to crabtalk daemon via Unix domain socket.
#[derive(Parser, Debug)]
#[command(name = "crabtalk", about = "Crabtalk CLI client")]
pub struct Cli {
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    /// Build a `RUST_LOG`-style filter string from the `-v` count on `daemon run`.
    ///
    /// Returns `None` when no `-v` flag is present (fall back to `RUST_LOG` env).
    pub fn log_filter(&self) -> Option<&'static str> {
        match self.command {
            Command::Daemon(ref d)
                if matches!(d.command, daemon::DaemonCommand::Run) && d.verbose > 0 =>
            {
                Some(match d.verbose {
                    1 => "crabtalk=info",
                    2 => "crabtalk=debug",
                    _ => "crabtalk=trace",
                })
            }
            Command::Hub(_) => Some("crabtalk=info"),
            _ => None,
        }
    }

    /// Parse and dispatch the CLI command.
    pub async fn run(self) -> Result<()> {
        match self.command {
            Command::Auth(cmd) => cmd.run().await,
            Command::Attach(cmd) => {
                let runner = connect_default_or_tcp(cmd.tcp).await?;
                cmd.run(runner).await
            }
            Command::Console(cmd) => {
                let runner = connect_default().await?;
                let selected = cmd.run(runner).await?;
                if let Some(path) = selected {
                    let runner = connect_default().await?;
                    let mut repl = crate::repl::ChatRepl::new(runner, "crab".into())?;
                    repl.resume(path).await
                } else {
                    Ok(())
                }
            }
            Command::Hub(cmd) => {
                let mut runner = connect_default().await?;
                cmd.run(&mut runner).await
            }
            #[cfg(unix)]
            Command::Daemon(cmd) => cmd.run(&wcore::paths::SOCKET_PATH).await,
            #[cfg(not(unix))]
            Command::Daemon(cmd) => cmd.run().await,
            Command::Ls => {
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
            Command::External(args) => external::run(args),
        }
    }
}

/// Top-level subcommands.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Attach to an agent via the interactive chat REPL.
    Attach(attach::Attach),
    /// Configure providers, models, and channel tokens interactively.
    Auth(auth::Auth),
    /// Interactive console for sessions and tasks.
    Console(console::Console),
    /// Install or uninstall hub packages.
    Hub(hub::Hub),
    /// Manage the crabtalk daemon (run, start, stop, reload).
    Daemon(daemon::Daemon),
    /// List running services.
    Ls,
    /// Forward to an external `crabtalk-{name}` binary (cargo-style).
    #[command(external_subcommand)]
    External(Vec<OsString>),
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
                "failed to connect to crabtalk daemon at {}. Is crabtalk daemon running?",
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
            "failed to read TCP port file at {}. Is crabtalk daemon running?",
            tcp_port_file.display()
        )
    })?;
    let port: u16 = port_str
        .trim()
        .parse()
        .with_context(|| format!("invalid port in {}", tcp_port_file.display()))?;
    Runner::connect_tcp(port).await.with_context(|| {
        format!("failed to connect to crabtalk daemon via TCP on port {port}. Is crabtalk daemon running?")
    })
}
