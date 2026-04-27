//! Transport wiring — UDS/TCP accept loops, protocol dispatch callback, shutdown bridging.

use super::Daemon;
use anyhow::Result;
use crabllm_core::Provider;
use futures_util::{StreamExt, pin_mut};
use std::path::Path;
use tokio::sync::{broadcast, mpsc, oneshot};
use wcore::protocol::{api::Server, message::ClientMessage};

fn dispatch_callback<P: Provider + 'static>(
    daemon: Daemon<P>,
) -> impl Fn(ClientMessage, mpsc::Sender<wcore::protocol::message::ServerMessage>) + Clone + Send + 'static
{
    move |msg, reply| {
        let daemon = daemon.clone();
        tokio::spawn(async move {
            let stream = daemon.dispatch(msg);
            pin_mut!(stream);
            while let Some(server_msg) = stream.next().await {
                if reply.send(server_msg).await.is_err() {
                    break;
                }
            }
        });
    }
}

#[cfg(unix)]
pub fn setup_socket<P: Provider + 'static>(
    daemon: Daemon<P>,
    shutdown_tx: &broadcast::Sender<()>,
) -> Result<(&'static Path, tokio::task::JoinHandle<()>)> {
    let resolved_path: &'static Path = &wcore::paths::SOCKET_PATH;
    if let Some(parent) = resolved_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if resolved_path.exists() {
        std::fs::remove_file(resolved_path)?;
    }

    let listener = tokio::net::UnixListener::bind(resolved_path)?;
    tracing::info!("daemon listening on {}", resolved_path.display());

    let socket_shutdown = bridge_shutdown(shutdown_tx.subscribe());
    let join = tokio::spawn(::transport::uds::accept_loop(
        listener,
        dispatch_callback(daemon),
        socket_shutdown,
    ));

    Ok((resolved_path, join))
}

pub fn setup_tcp<P: Provider + 'static>(
    daemon: Daemon<P>,
    shutdown_tx: &broadcast::Sender<()>,
) -> Result<(tokio::task::JoinHandle<()>, u16)> {
    let (std_listener, addr) = ::transport::tcp::bind()?;
    let listener = tokio::net::TcpListener::from_std(std_listener)?;
    tracing::info!("daemon listening on tcp://{addr}");

    let tcp_shutdown = bridge_shutdown(shutdown_tx.subscribe());
    let join = tokio::spawn(::transport::tcp::accept_loop(
        listener,
        dispatch_callback(daemon),
        tcp_shutdown,
    ));

    Ok((join, addr.port()))
}

pub fn bridge_shutdown(mut rx: broadcast::Receiver<()>) -> oneshot::Receiver<()> {
    let (otx, orx) = oneshot::channel();
    tokio::spawn(async move {
        let _ = rx.recv().await;
        let _ = otx.send(());
    });
    orx
}
