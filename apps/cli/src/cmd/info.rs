//! What the daemon is holding right now.

use crate::{conn, table};
use anyhow::Result;
use proto::api::Client as _;

pub async fn run() -> Result<()> {
    let mut transport = conn::connect().await?;
    let stats = transport.get_stats().await?;

    println!("Version:         {}", stats.version);
    println!("Uptime:          {}", table::duration(stats.uptime_secs));
    println!("Agents:          {}", stats.registered_agents);
    println!("Conversations:   {}", stats.active_conversations);
    println!("Active model:    {}", stats.active_model);
    Ok(())
}
