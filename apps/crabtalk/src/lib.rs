//! Crabtalk daemon — CLI dispatch and daemon lifecycle.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use futures_util::StreamExt;
use std::{collections::HashMap, ffi::OsString};
use transport::Transport;
use wcore::protocol::{
    api::Client,
    message::{AgentEventKind, plugin_event},
};

mod attach;
mod external;
mod foreground;
mod service;

/// Crabtalk — AI agent platform.
#[derive(Parser, Debug)]
#[command(name = "crabtalk", about = "Crabtalk — AI agent daemon")]
pub struct Cli {
    /// Run the daemon in the foreground.
    #[arg(long)]
    pub foreground: bool,
    /// Increase log verbosity (-v = info, -vv = debug, -vvv = trace).
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
    /// Connect via TCP instead of Unix domain socket.
    #[arg(long)]
    pub tcp: bool,
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Top-level subcommands.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Install and start the daemon service.
    Start {
        /// Re-install even if already running.
        #[arg(long)]
        force: bool,
    },
    /// Stop the daemon service.
    Stop,
    /// Restart the daemon service.
    Restart,
    /// Hot-reload daemon config.
    Reload,
    /// Stream daemon events.
    Events,
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
    /// List running services.
    Ps,
    /// View daemon logs.
    Logs {
        /// Arguments passed through to `tail` (e.g. `-f`, `-n 100`).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        tail_args: Vec<String>,
    },
    /// Forward to an external `crabtalk-{name}` binary.
    #[command(external_subcommand)]
    External(Vec<OsString>),
}

impl Cli {
    pub async fn run(self) -> Result<()> {
        if self.foreground {
            return foreground::start().await;
        }

        match self.command {
            None => exec_default_client(),
            Some(Command::Start { force }) => {
                ensure_config()?;
                service::install(self.verbose.max(1), force)
            }
            Some(Command::Stop) => service::uninstall(),
            Some(Command::Restart) => {
                let _ = service::uninstall();
                ensure_config()?;
                service::install(self.verbose.max(1), true)
            }
            Some(Command::Reload) => {
                let mut conn = connect(self.tcp).await?;
                use wcore::protocol::message::{ClientMessage, client_message};
                let msg = ClientMessage {
                    msg: Some(client_message::Msg::Reload(Default::default())),
                };
                conn.request(msg).await?;
                println!("daemon reloaded");
                Ok(())
            }
            Some(Command::Events) => stream_events(connect(self.tcp).await?).await,
            Some(Command::Pull { plugin, force }) => {
                use std::io::Write;
                let mut conn = connect(self.tcp).await?;
                let stream =
                    conn.install_plugin(plugin.clone(), String::new(), String::new(), force);
                tokio::pin!(stream);
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
                let mut conn = connect(self.tcp).await?;
                let stream = conn.uninstall_plugin(plugin.clone());
                tokio::pin!(stream);
                while let Some(event) = stream.next().await {
                    match event? {
                        plugin_event::Event::Step(s) => println!("  {}", s.message),
                        plugin_event::Event::Warning(w) => {
                            eprintln!("  warning: {}", w.message);
                        }
                        plugin_event::Event::Done(d) => {
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

/// Scaffold config dir and prompt for provider if none configured.
fn ensure_config() -> Result<()> {
    node::storage::scaffold_config_dir(&wcore::paths::CONFIG_DIR)?;
    let config_path = wcore::paths::CONFIG_DIR.join(wcore::paths::CONFIG_FILE);
    let config = node::NodeConfig::load(&config_path)?;
    if config.provider.is_empty() {
        attach::setup_provider(&config_path)?;
    }
    Ok(())
}

/// Exec the default client binary (crabtalk-tui) if available.
fn exec_default_client() -> Result<()> {
    if let Some(path) = external::find_binary("crabtalk-tui") {
        let status = std::process::Command::new(&path)
            .args(std::env::args_os().skip(1))
            .status()
            .with_context(|| format!("failed to run {}", path.display()))?;
        std::process::exit(status.code().unwrap_or(1));
    }
    println!("crabtalk daemon — no client installed\n");
    println!("Available commands:");
    println!("  crabtalk start       Install and start the daemon service");
    println!("  crabtalk foreground   Run the daemon in the foreground");
    println!("  crabtalk stop        Stop the daemon service");
    println!("  crabtalk reload      Hot-reload daemon config");
    println!("  crabtalk pull <pkg>  Install a plugin");
    println!("  crabtalk rm <pkg>    Uninstall a plugin");
    println!("  crabtalk ps          List running services");
    println!("  crabtalk logs        View daemon logs");
    println!("\nInstall a client:");
    println!("  cargo install crabtalk-tui");
    Ok(())
}

/// Stream daemon events to stdout, buffering text/thinking deltas.
async fn stream_events(mut conn: Transport) -> Result<()> {
    use wcore::protocol::message::{
        ClientMessage, SubscribeEvents, client_message, server_message,
    };
    let stream = conn.request_stream(ClientMessage {
        msg: Some(client_message::Msg::SubscribeEvents(SubscribeEvents {})),
    });
    tokio::pin!(stream);
    let mut buffers: HashMap<(String, String), String> = HashMap::new();
    while let Some(Ok(msg)) = stream.next().await {
        if let Some(server_message::Msg::AgentEvent(event)) = msg.msg {
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
                        let trimmed = truncate(&text, 80);
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
    }
    Ok(())
}

/// Connect to the running daemon via UDS (default) or TCP.
async fn connect(use_tcp: bool) -> Result<Transport> {
    if use_tcp {
        connect_tcp().await
    } else {
        connect_default().await
    }
}

async fn connect_default() -> Result<Transport> {
    #[cfg(unix)]
    {
        let socket_path = &*wcore::paths::SOCKET_PATH;
        let config = transport::uds::ClientConfig {
            socket_path: socket_path.clone(),
        };
        let conn = transport::uds::CrabtalkClient::new(config)
            .connect()
            .await
            .with_context(|| format!("failed to connect to daemon at {}", socket_path.display()))?;
        Ok(Transport::Uds(conn))
    }
    #[cfg(not(unix))]
    {
        connect_tcp().await
    }
}

async fn connect_tcp() -> Result<Transport> {
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
    let addr = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port));
    let conn = transport::tcp::TcpConnection::connect(addr)
        .await
        .with_context(|| format!("failed to connect to daemon via TCP on port {port}"))?;
    Ok(Transport::Tcp(conn))
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max.saturating_sub(3);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}
