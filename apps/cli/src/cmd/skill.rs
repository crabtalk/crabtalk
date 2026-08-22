//! Skills.

use crate::{conn, table};
use anyhow::Result;
use proto::api::Client as _;

#[derive(clap::Subcommand, Debug)]
pub enum Command {
    /// List skills.
    Ls,
    /// Print a skill's body.
    Inspect {
        /// Skill name.
        name: String,
    },
}

impl Command {
    pub async fn run(self) -> Result<()> {
        let mut transport = conn::connect().await?;
        match self {
            Self::Ls => {
                let rows = transport
                    .list_skills()
                    .await?
                    .into_iter()
                    .map(|s| vec![s.name, table::truncate(&s.description, 60)])
                    .collect::<Vec<_>>();
                table::print(&["NAME", "DESCRIPTION"], &rows);
            }
            Self::Inspect { name } => println!("{}", transport.get_skill(name).await?.body),
        }
        Ok(())
    }
}
