//! Models the configured endpoint serves.

use crate::{conn, table};
use anyhow::Result;
use proto::api::Client as _;

#[derive(clap::Subcommand, Debug)]
pub enum Command {
    /// List models.
    Ls,
}

impl Command {
    pub async fn run(self) -> Result<()> {
        let Self::Ls = self;
        let mut transport = conn::connect().await?;
        let rows = transport
            .list_models()
            .await?
            .into_iter()
            .map(|m| {
                let active = if m.active { "*" } else { "" };
                vec![m.name, active.to_owned()]
            })
            .collect::<Vec<_>>();
        table::print(&["NAME", "ACTIVE"], &rows);
        Ok(())
    }
}
