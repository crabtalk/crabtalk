//! `crabtalk mcp` — non-interactive MCP server CRUD scoped to an agent.

use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::PathBuf;
use wcore::protocol::api::Client;

/// Manage MCP servers declared by an agent.
#[derive(Args, Debug)]
pub struct Mcp {
    #[command(subcommand)]
    pub command: McpCmd,
}

#[derive(Subcommand, Debug)]
pub enum McpCmd {
    /// List MCP servers declared by agents. With `--agent`, scope to one.
    List {
        /// Agent name. Empty = union view across every agent.
        #[arg(long, default_value = "")]
        agent: String,
    },
    /// Add or replace an MCP in the given agent's `mcps` list.
    Create {
        /// Agent name.
        #[arg(long)]
        agent: String,
        /// Path to JSON config file. Use `-` to read from stdin.
        #[arg(long)]
        config: PathBuf,
    },
    /// Remove an MCP from the given agent's `mcps` list.
    Delete {
        /// Agent name.
        #[arg(long)]
        agent: String,
        /// MCP server name.
        name: String,
    },
}

impl Mcp {
    pub async fn run(self, tcp: bool) -> Result<()> {
        let (mut runner, _) = super::connect(tcp).await?;
        match self.command {
            McpCmd::List { agent } => {
                let mcps = runner.list_mcps(agent).await?;
                if mcps.is_empty() {
                    return Ok(());
                }
                let name_w = mcps.iter().map(|m| m.name.len()).max().unwrap_or(0);
                let src_w = mcps.iter().map(|m| m.source.len()).max().unwrap_or(0);
                for m in mcps {
                    println!("{:<name_w$}  {:<src_w$}  {}", m.name, m.source, m.command);
                }
            }
            McpCmd::Create { agent, config } => {
                let json = super::read_path_or_stdin(&config)?;
                let info = runner.upsert_mcp(agent, json).await?;
                println!("saved '{}'", info.name);
            }
            McpCmd::Delete { agent, name } => {
                runner.delete_mcp(agent, name).await?;
            }
        }
        Ok(())
    }
}
