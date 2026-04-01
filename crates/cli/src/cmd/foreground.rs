//! `crabtalk --foreground` — run the daemon in the foreground.

use anyhow::Result;
use wcore::paths::{CONFIG_DIR, TCP_PORT_FILE};

pub async fn start() -> Result<()> {
    daemon::config::scaffold_config_dir(&CONFIG_DIR)?;

    // Check if providers are configured; prompt if empty.
    let config_path = CONFIG_DIR.join(wcore::paths::CONFIG_FILE);
    let config = daemon::DaemonConfig::load(&config_path)?;
    if config.provider.is_empty() {
        crate::cmd::attach::setup_provider(&config_path)?;
    }

    let handle = daemon::Daemon::start(&CONFIG_DIR).await?;

    // UDS transport (Unix only).
    #[cfg(unix)]
    let (socket_path, socket_join) = daemon::setup_socket(&handle.shutdown_tx, &handle.event_tx)?;
    #[cfg(unix)]
    tracing::info!("crabtalk daemon listening on {}", socket_path.display());

    // TCP transport.
    let (tcp_join, tcp_port) = daemon::setup_tcp(&handle.shutdown_tx, &handle.event_tx)?;
    std::fs::write(&*TCP_PORT_FILE, tcp_port.to_string())?;
    tracing::info!("wrote tcp port file at {}", TCP_PORT_FILE.display());

    handle.wait_until_ready().await?;

    tokio::signal::ctrl_c().await?;
    tracing::info!("received ctrl-c, shutting down");

    let grace = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        handle.shutdown().await?;
        #[cfg(unix)]
        socket_join.await?;
        tcp_join.await?;
        anyhow::Ok(())
    });
    if grace.await.is_err() {
        tracing::warn!("graceful shutdown timed out, forcing exit");
    }
    #[cfg(unix)]
    let _ = std::fs::remove_file(socket_path);
    let _ = std::fs::remove_file(&*TCP_PORT_FILE);
    tracing::info!("crabtalk daemon shut down");
    std::process::exit(0)
}
