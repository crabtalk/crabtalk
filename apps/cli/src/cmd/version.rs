//! Both halves of the install, so a skew between them is visible.

use crate::conn;
use anyhow::Result;
use proto::api::Client as _;

pub async fn run() -> Result<()> {
    let client = env!("CARGO_PKG_VERSION");
    println!("Client:");
    println!(" Version:  {client}");

    println!("Server:");
    match conn::connect().await {
        Ok(mut transport) => {
            let stats = transport.get_stats().await?;
            println!(" Version:  {}", stats.version);
            println!(" Uptime:   {}", crate::table::duration(stats.uptime_secs));
            if stats.version != client {
                println!();
                println!("warning: client and daemon disagree — reinstall with `crabup install`");
            }
        }
        Err(e) => println!(" {e}"),
    }
    Ok(())
}
