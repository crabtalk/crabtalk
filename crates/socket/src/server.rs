//! Unix domain socket server — accept loop and per-connection message handler.

use tokio::{
    net::UnixListener,
    sync::{mpsc, oneshot},
};
use wcore::protocol::{
    codec,
    message::{ClientMessage, ServerMessage},
};

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
    F: Fn(ClientMessage, mpsc::UnboundedSender<ServerMessage>) + Clone + Send + 'static,
{
    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, _addr)) => {
                        let cb = on_message.clone();
                        tokio::spawn(async move {
                            let (mut reader, mut writer) = stream.into_split();
                            let (tx, mut rx) = mpsc::unbounded_channel::<ServerMessage>();
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
                                    Err(e) => { tracing::debug!("read error: {e}"); break; }
                                };
                                cb(client_msg, tx.clone());
                            }

                            drop(tx);
                            let _ = send_task.await;
                        });
                    }
                    Err(e) => tracing::error!("failed to accept connection: {e}"),
                }
            }
            _ = &mut shutdown => {
                tracing::info!("accept loop shutting down");
                break;
            }
        }
    }
}
