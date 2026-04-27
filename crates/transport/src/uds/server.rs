//! Unix domain socket server — accept loop and per-connection message handler.

use crate::REPLY_CHANNEL_CAPACITY;
use std::time::Duration;
use tokio::{
    net::UnixListener,
    sync::{mpsc, oneshot},
};
use wcore::protocol::{
    codec,
    message::{ClientMessage, ServerMessage},
};

/// Backoff after an `accept()` error so EMFILE / ENFILE / etc. don't
/// hot-spin the loop and flood the logs.
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(100);

/// Accept connections on the given `UnixListener` until shutdown is signalled.
///
/// Each connection is handled in a separate task. For each incoming
/// `ClientMessage`, calls `on_message(msg, reply_tx)` where `reply_tx` is
/// the per-connection sender for streaming `ServerMessage`s back.
pub async fn accept_loop<F>(
    listener: UnixListener,
    on_message: F,
    mut shutdown: oneshot::Receiver<()>,
) where
    F: Fn(ClientMessage, mpsc::Sender<ServerMessage>) + Clone + Send + 'static,
{
    loop {
        let (stream, _addr) = tokio::select! {
            result = listener.accept() => match result {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::error!("failed to accept connection: {e}");
                    tokio::time::sleep(ACCEPT_ERROR_BACKOFF).await;
                    continue;
                }
            },
            _ = &mut shutdown => {
                tracing::info!("accept loop shutting down");
                break;
            }
        };

        let cb = on_message.clone();
        tokio::spawn(async move {
            let (mut reader, mut writer) = stream.into_split();
            let (tx, mut rx) = mpsc::channel::<ServerMessage>(REPLY_CHANNEL_CAPACITY);
            let send_task = tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    if let Err(e) = codec::write_message(&mut writer, &msg).await {
                        tracing::error!("failed to write message: {e}");
                        break;
                    }
                }
            });

            loop {
                let client_msg: ClientMessage = match codec::read_message(&mut reader).await {
                    Ok(msg) => msg,
                    Err(codec::FrameError::ConnectionClosed) => break,
                    Err(e) => {
                        tracing::debug!("read error: {e}");
                        break;
                    }
                };
                cb(client_msg, tx.clone());
            }

            // Client is gone — drop our tx and abort send_task so
            // the writer half releases the socket FD immediately.
            // Without abort, send_task would await rx.recv() until
            // every dispatch task drops its tx clone (which for
            // long-lived subscriptions never happens), leaking the
            // FD for the lifetime of the daemon.
            drop(tx);
            send_task.abort();
            let _ = send_task.await;
        });
    }
}
