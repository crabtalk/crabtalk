//! Crabtalk daemon — service definition and foreground startup.

use anyhow::Result;

pub mod foreground;

#[command::command(kind = "client", name = "daemon")]
pub struct Daemon;

impl Daemon {
    async fn run(&self) -> anyhow::Result<()> {
        ensure_config()?;
        foreground::start().await
    }
}

fn ensure_config() -> Result<()> {
    crabtalk::storage::scaffold_config_dir(&wcore::paths::CONFIG_DIR)?;
    Ok(())
}
