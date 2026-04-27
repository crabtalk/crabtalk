//! `crabtalk mcp` — non-interactive MCP server CRUD.

use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::PathBuf;
use wcore::protocol::api::Client;

/// Manage MCP servers.
#[derive(Args, Debug)]
pub struct Mcp {
    #[command(subcommand)]
    pub command: McpCmd,
}

#[derive(Subcommand, Debug)]
pub enum McpCmd {
    /// List configured MCP servers.
    List,
    /// Create or replace an MCP server from a JSON `McpServerConfig`.
    Create {
        /// Path to JSON config file. Use `-` to read from stdin.
        #[arg(long)]
        config: PathBuf,
    },
    /// Delete an MCP server by name.
    Delete {
        /// MCP server name.
        name: String,
    },
}

impl Mcp {
    pub async fn run(self, tcp: bool) -> Result<()> {
        let (mut runner, _) = super::connect(tcp).await?;
        match self.command {
            McpCmd::List => {
                let mcps = runner.list_mcps().await?;
                if mcps.is_empty() {
                    return Ok(());
                }
                let name_w = mcps.iter().map(|m| m.name.len()).max().unwrap_or(0);
                let src_w = mcps.iter().map(|m| m.source.len()).max().unwrap_or(0);
                for m in mcps {
                    println!("{:<name_w$}  {:<src_w$}  {}", m.name, m.source, m.command,);
                }
            }
            McpCmd::Create { config } => {
                let json = super::read_path_or_stdin(&config)?;
                let info = runner.upsert_mcp(json).await?;
                println!("saved '{}'", info.name);
            }
            McpCmd::Delete { name } => {
                runner.delete_mcp(name).await?;
            }
        }
        Ok(())
    }
}
