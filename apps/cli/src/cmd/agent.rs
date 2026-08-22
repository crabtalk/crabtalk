//! Agents.

use crate::{conn, table};
use anyhow::Result;
use proto::api::Client as _;

#[derive(clap::Subcommand, Debug)]
pub enum Command {
    /// List agents.
    Ls,
    /// Print an agent's configuration.
    Inspect {
        /// Agent name.
        name: String,
    },
}

impl Command {
    pub async fn run(self) -> Result<()> {
        let mut transport = conn::connect().await?;
        match self {
            Self::Ls => {
                let rows = transport
                    .list_agents()
                    .await?
                    .into_iter()
                    .map(|a| {
                        vec![
                            a.id,
                            a.name,
                            a.model,
                            a.skills.len().to_string(),
                            a.mcps.len().to_string(),
                            table::truncate(&a.description, 40),
                        ]
                    })
                    .collect::<Vec<_>>();
                table::print(
                    &["ID", "NAME", "MODEL", "SKILLS", "MCPS", "DESCRIPTION"],
                    &rows,
                );
            }
            // The daemon already holds the config as JSON, so inspect is
            // what it hands over rather than a rendering of it.
            Self::Inspect { name } => println!("{}", transport.get_agent(name).await?.config),
        }
        Ok(())
    }
}
