//! Crabtalk daemon — CLI dispatch and daemon lifecycle.

use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use futures_util::StreamExt;
use std::collections::HashMap;
use transport::Transport;
use wcore::protocol::{
    api::Client,
    message::{AgentEventKind, McpEventKind, plugin_event},
};

pub mod attach;
pub mod foreground;

/// Crabtalk — AI agent platform.
#[derive(Parser, Debug)]
#[command(name = "crabtalk", about = "Crabtalk — AI agent daemon")]
pub struct Cli {
    /// Run the daemon in the foreground (legacy alias for `run`).
    #[arg(long)]
    pub foreground: bool,
    /// Increase log verbosity (-v = info, -vv = debug, -vvv = trace).
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
    /// Connect via TCP instead of Unix domain socket.
    #[arg(long)]
    pub tcp: bool,
    /// Subcommand to execute. Optional so `--foreground` can run the
    /// daemon without one (legacy launchd/systemd units invoke us as
    /// `crabtalkd --foreground -v`).
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Top-level subcommands.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the daemon in the foreground (what launchd/systemd invokes).
    Run,
    /// Scaffold config and prompt for LLM endpoint on first run.
    Setup,
    /// Hot-reload daemon config.
    Reload,
    /// Stream daemon events.
    Events,
    /// Stream MCP server lifecycle events.
    McpEvents,
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
}

impl Cli {
    pub async fn run(self) -> Result<()> {
        if self.foreground {
            return foreground::start().await;
        }

        let Some(command) = self.command else {
            Self::command().print_help()?;
            std::process::exit(2);
        };

        match command {
            Command::Run => foreground::start().await,
            Command::Setup => ensure_config(),
            Command::Reload => {
                let mut conn = connect(self.tcp).await?;
                use wcore::protocol::message::{ClientMessage, client_message};
                let msg = ClientMessage {
                    msg: Some(client_message::Msg::Reload(Default::default())),
                };
                conn.request(msg).await?;
                println!("daemon reloaded");
                Ok(())
            }
            Command::Events => stream_events(connect(self.tcp).await?).await,
            Command::McpEvents => stream_mcp_events(connect(self.tcp).await?).await,
            Command::Pull { plugin, force } => {
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
            Command::Rm { plugin } => {
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
        }
    }
}

/// Scaffold config dir and prompt for LLM endpoint if none configured.
fn ensure_config() -> Result<()> {
    crabtalk::storage::scaffold_config_dir(&wcore::paths::CONFIG_DIR)?;
    let config_path = wcore::paths::CONFIG_DIR.join(wcore::paths::CONFIG_FILE);
    let config = crabtalk::DaemonConfig::load(&config_path)?;
    if config.llm.base_url.is_empty() {
        attach::setup_llm(&config_path)?;
    }
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

/// Stream MCP lifecycle events to stdout.
async fn stream_mcp_events(mut conn: Transport) -> Result<()> {
    let stream = conn.subscribe_mcp_events();
    tokio::pin!(stream);
    while let Some(result) = stream.next().await {
        let event = result?;
        let kind = match McpEventKind::try_from(event.kind) {
            Ok(McpEventKind::Connecting) => "connecting",
            Ok(McpEventKind::Connected) => "connected",
            Ok(McpEventKind::Failed) => "failed",
            Ok(McpEventKind::Disconnected) => "disconnected",
            _ => "unknown",
        };
        let detail = match McpEventKind::try_from(event.kind) {
            Ok(McpEventKind::Connected) => format!(" ({} tool(s))", event.tools.len()),
            Ok(McpEventKind::Failed) if !event.error.is_empty() => format!(": {}", event.error),
            _ => String::new(),
        };
        println!(
            "[{}] {} mcp/{} {}{}",
            event.timestamp, event.agent, event.name, kind, detail
        );
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
