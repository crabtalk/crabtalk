//! Where this daemon answers — a UDS socket and a TCP port, both under the
//! install root.
//!
//! Which endpoints a process serves, and where it advertises them, is the
//! process's own decision: the library is handed a message and returns a
//! stream, and knows nothing about this machine's layout.

use crate::backend::Backend;
use anyhow::Result;
use crabtalk::{CrabTalk, system::provider::DefaultProvider};
use futures_util::{StreamExt, pin_mut};
use proto::{ClientMessage, ServerMessage, server::Server};
#[cfg(unix)]
use std::path::Path;
use tokio::sync::{broadcast, mpsc, oneshot};

/// This daemon's instantiation of the library.
type Daemon = CrabTalk<DefaultProvider, Backend>;

fn on_message(
    daemon: Daemon,
) -> impl Fn(ClientMessage, mpsc::Sender<ServerMessage>) + Clone + Send {
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
pub fn socket(
    daemon: Daemon,
    shutdown_tx: &broadcast::Sender<()>,
) -> Result<(&'static Path, tokio::task::JoinHandle<()>)> {
    let resolved_path: &'static Path = &crabup::dirs::SOCKET_PATH;
    if let Some(parent) = resolved_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if resolved_path.exists() {
        std::fs::remove_file(resolved_path)?;
    }

    let listener = tokio::net::UnixListener::bind(resolved_path)?;
    tracing::info!("daemon listening on {}", resolved_path.display());

    let join = tokio::spawn(transport::uds::accept_loop(
        listener,
        on_message(daemon),
        bridge_shutdown(shutdown_tx.subscribe()),
    ));

    Ok((resolved_path, join))
}

pub fn tcp(
    daemon: Daemon,
    shutdown_tx: &broadcast::Sender<()>,
) -> Result<(tokio::task::JoinHandle<()>, u16)> {
    let (std_listener, addr) = transport::tcp::bind()?;
    let listener = tokio::net::TcpListener::from_std(std_listener)?;
    tracing::info!("daemon listening on tcp://{addr}");

    let join = tokio::spawn(transport::tcp::accept_loop(
        listener,
        on_message(daemon),
        bridge_shutdown(shutdown_tx.subscribe()),
    ));

    Ok((join, addr.port()))
}

/// The accept loops take a one-shot, the daemon broadcasts to everything.
fn bridge_shutdown(mut rx: broadcast::Receiver<()>) -> oneshot::Receiver<()> {
    let (otx, orx) = oneshot::channel();
    tokio::spawn(async move {
        let _ = rx.recv().await;
        let _ = otx.send(());
    });
    orx
}
