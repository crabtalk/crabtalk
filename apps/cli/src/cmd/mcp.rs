//! MCP servers, as declared by agents.

use crate::{conn, table};
use anyhow::Result;
use proto::api::Client as _;

#[derive(clap::Subcommand, Debug)]
pub enum Command {
    /// List MCP servers.
    Ls {
        /// One agent's, rather than every agent's.
        #[arg(long)]
        agent: Option<String>,
    },
}

impl Command {
    pub async fn run(self) -> Result<()> {
        let Self::Ls { agent } = self;
        let mut transport = conn::connect().await?;
        let rows = transport
            .list_mcps(agent.unwrap_or_default())
            .await?
            .into_iter()
            .map(|m| {
                let transport = match m.url.is_empty() {
                    true => m.command,
                    false => m.url,
                };
                vec![m.name, transport, m.source]
            })
            .collect::<Vec<_>>();
        table::print(&["NAME", "TRANSPORT", "SOURCE"], &rows);
        Ok(())
    }
}
