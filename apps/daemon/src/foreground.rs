//! Run the daemon in the foreground with live logging.

use anyhow::Result;
use wcore::paths::{CONFIG_DIR, SOCKET_PATH, TCP_PORT_FILE};

pub async fn start() -> Result<()> {
    crate::ensure_config()?;
    let handle = crabtalk::Node::start(&CONFIG_DIR).await?;

    #[cfg(unix)]
    let (socket_path, socket_join) =
        crabtalk::setup_socket(handle.node.clone(), &handle.shutdown_tx)?;

    let (tcp_join, tcp_port) = crabtalk::setup_tcp(handle.node.clone(), &handle.shutdown_tx)?;
    std::fs::write(&*TCP_PORT_FILE, tcp_port.to_string())?;
    tracing::info!(tcp_port, "TCP transport listening");

    handle.wait_until_ready().await?;
    tracing::info!("daemon ready");

    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down…");
    handle.shutdown().await?;

    let timeout = std::time::Duration::from_secs(5);
    #[cfg(unix)]
    {
        let _ = tokio::time::timeout(timeout, socket_join).await;
        let _ = std::fs::remove_file(&*SOCKET_PATH);
        tracing::info!(path = %socket_path.display(), "removed socket");
    }
    let _ = tokio::time::timeout(timeout, tcp_join).await;
    let _ = std::fs::remove_file(&*TCP_PORT_FILE);
    Ok(())
}
