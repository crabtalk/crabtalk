//! `walrus daemon` — start the walrus daemon in the foreground.

use anyhow::Result;
use clap::Args;
use daemon::{Daemon as WalrusDaemon, config};
use wcore::paths::{CONFIG_DIR, TCP_PORT_FILE};

/// Start the walrus daemon in the foreground.
#[derive(Args, Debug)]
pub struct Daemon;

impl Daemon {
    /// Run the daemon, blocking until Ctrl-C.
    pub async fn run(self) -> Result<()> {
        if !CONFIG_DIR.exists() {
            config::scaffold_config_dir(&CONFIG_DIR)?;
            tracing::info!("created config directory at {}", CONFIG_DIR.display());
        }

        let handle = WalrusDaemon::start(&CONFIG_DIR).await?;

        // UDS transport.
        let (socket_path, socket_join) =
            daemon::setup_socket(&handle.shutdown_tx, &handle.event_tx)?;
        tracing::info!("walrusd listening on {}", socket_path.display());

        // TCP transport.
        let (tcp_join, tcp_port) = daemon::setup_tcp(&handle.shutdown_tx, &handle.event_tx)?;
        std::fs::write(&*TCP_PORT_FILE, tcp_port.to_string())?;
        tracing::info!("wrote tcp port file at {}", TCP_PORT_FILE.display());

        handle.wait_until_ready().await?;

        tokio::signal::ctrl_c().await?;
        tracing::info!("received ctrl-c, shutting down");
        handle.shutdown().await?;
        socket_join.await?;
        tcp_join.await?;
        let _ = std::fs::remove_file(socket_path);
        let _ = std::fs::remove_file(&*TCP_PORT_FILE);
        tracing::info!("walrusd shut down");
        Ok(())
    }
}
