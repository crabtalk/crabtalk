//! The daemon: a store, the runtime over it, and the sockets it answers on.

use crate::backend::{Backend, STORE_PATH};
use crate::serve;
use anyhow::Result;
use crabtalk::CrabTalk;
use std::{sync::Arc, time::Duration};

/// How long a listener gets to drain after shutdown is broadcast.
const DRAIN: Duration = Duration::from_secs(5);

pub async fn start() -> Result<()> {
    let store = Arc::new(Backend::open(&*STORE_PATH)?);
    tracing::info!("store at {}", STORE_PATH.display());

    let handle = CrabTalk::start(&crabup::CONFIG_DIR, store.clone()).await?;

    #[cfg(unix)]
    let (socket, socket_join) = serve::socket(handle.inner.clone(), &handle.shutdown_tx)?;
    let (tcp_join, port) = serve::tcp(handle.inner.clone(), &handle.shutdown_tx)?;
    std::fs::write(&*crabup::dirs::PORT_FILE, port.to_string())?;

    handle.wait_until_ready().await?;
    tracing::info!("daemon ready");

    terminate().await?;
    tracing::info!("shutting down");
    handle.shutdown().await?;

    #[cfg(unix)]
    {
        let _ = tokio::time::timeout(DRAIN, socket_join).await;
        let _ = std::fs::remove_file(socket);
    }
    let _ = tokio::time::timeout(DRAIN, tcp_join).await;
    let _ = std::fs::remove_file(&*crabup::dirs::PORT_FILE);

    store.checkpoint()
}

/// SIGTERM is how whatever spawned this kills it; ctrl-c is the terminal case.
#[cfg(unix)]
async fn terminate() -> Result<()> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut term = signal(SignalKind::terminate())?;
    tokio::select! {
        signal = tokio::signal::ctrl_c() => signal?,
        _ = term.recv() => {}
    }
    Ok(())
}

#[cfg(not(unix))]
async fn terminate() -> Result<()> {
    Ok(tokio::signal::ctrl_c().await?)
}
