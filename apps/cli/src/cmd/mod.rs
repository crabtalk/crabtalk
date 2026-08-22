//! The commands — one file per noun, each a call and a print.

use anyhow::Result;

pub mod agent;
pub mod info;
pub mod logs;
pub mod mcp;
pub mod model;
pub mod ps;
pub mod skill;
pub mod version;

#[derive(clap::Parser, Debug)]
#[command(name = "crabtalk", about = "The Crabtalk command line", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand, Debug)]
pub enum Command {
    /// Show the client and daemon versions.
    Version,
    /// Show daemon-wide information.
    Info,
    /// List conversations.
    Ps {
        /// Include conversations that are no longer live.
        #[arg(short, long)]
        all: bool,
    },
    /// Show a conversation's messages, or follow the daemon's events.
    Logs {
        /// Session handle, as printed by `crabtalk ps`.
        handle: Option<String>,
        /// Follow live events instead of printing stored messages.
        #[arg(short, long)]
        follow: bool,
    },
    /// Agents.
    Agent {
        #[command(subcommand)]
        command: agent::Command,
    },
    /// Models the endpoint serves.
    Model {
        #[command(subcommand)]
        command: model::Command,
    },
    /// MCP servers.
    Mcp {
        #[command(subcommand)]
        command: mcp::Command,
    },
    /// Skills.
    Skill {
        #[command(subcommand)]
        command: skill::Command,
    },
}

impl Cli {
    pub async fn run(self) -> Result<()> {
        match self.command {
            Command::Version => version::run().await,
            Command::Info => info::run().await,
            Command::Ps { all } => ps::run(all).await,
            Command::Logs { handle, follow } => logs::run(handle, follow).await,
            Command::Agent { command } => command.run().await,
            Command::Model { command } => command.run().await,
            Command::Mcp { command } => command.run().await,
            Command::Skill { command } => command.run().await,
        }
    }
}
