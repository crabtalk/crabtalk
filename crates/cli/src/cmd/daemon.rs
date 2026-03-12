//! `walrus daemon` — start the walrus daemon in the foreground.

use anyhow::Result;
use clap::Args;
use daemon::{Daemon as WalrusDaemon, config};
use wcore::paths::CONFIG_DIR;

/// Start the walrus daemon in the foreground.
#[derive(Args, Debug)]
pub struct Daemon {
    /// Listen on TCP instead of Unix domain socket.
    #[arg(long)]
    pub tcp: Option<std::net::SocketAddr>,
}

impl Daemon {
    /// Run the daemon, blocking until Ctrl-C.
    pub async fn run(self) -> Result<()> {
        if !CONFIG_DIR.exists() {
            config::scaffold_config_dir(&CONFIG_DIR)?;
            tracing::info!("created config directory at {}", CONFIG_DIR.display());
        }

        let handle = WalrusDaemon::start(&CONFIG_DIR).await?;

        // Spawn transport: TCP or UDS (mutually exclusive).
        let mut socket_path = None;
        let transport_join = if let Some(addr) = self.tcp {
            daemon::setup_tcp(addr, &handle.shutdown_tx, &handle.event_tx)?
        } else {
            let (path, join) = daemon::setup_socket(&handle.shutdown_tx, &handle.event_tx)?;
            tracing::info!("walrusd listening on {}", path.display());
            socket_path = Some(path);
            join
        };

        daemon::setup_channels(&handle.config, &handle.event_tx).await;
        handle.wait_until_ready().await?;

        tokio::signal::ctrl_c().await?;
        tracing::info!("received ctrl-c, shutting down");
        handle.shutdown().await?;
        transport_join.await?;
        if let Some(path) = socket_path {
            let _ = std::fs::remove_file(path);
        }
        tracing::info!("walrusd shut down");
        Ok(())
    }
}
