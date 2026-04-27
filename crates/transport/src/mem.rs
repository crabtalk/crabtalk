//! In-process transport — a [`Client`] backed by direct dispatch into a
//! [`Server`] held in the same process. No serialization, no socket.
//!
//! Use this when crabtalk is embedded as a library: the consumer constructs
//! the daemon (a `Server` impl), calls [`connect`] to obtain a
//! [`MemConnection`], then drives it through the [`Client`] trait.

use crate::REPLY_CHANNEL_CAPACITY;
use anyhow::Result;
use futures_core::Stream;
use futures_util::StreamExt;
use std::sync::Arc;
use tokio::sync::mpsc;
use wcore::protocol::{
    api::{Client, Server},
    message::{ClientMessage, ServerMessage},
};

/// In-process connection — a pair of channels into the embedded daemon.
pub struct MemConnection {
    tx: mpsc::Sender<ClientMessage>,
    rx: mpsc::Receiver<ServerMessage>,
}

impl Client for MemConnection {
    async fn request(&mut self, msg: ClientMessage) -> Result<ServerMessage> {
        self.tx
            .send(msg)
            .await
            .map_err(|_| anyhow::anyhow!("mem connection closed"))?;
        self.rx
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("mem connection closed"))
    }

    fn request_stream(
        &mut self,
        msg: ClientMessage,
    ) -> impl Stream<Item = Result<ServerMessage>> + Send + '_ {
        async_stream::try_stream! {
            self.tx
                .send(msg)
                .await
                .map_err(|_| anyhow::anyhow!("mem connection closed"))?;
            loop {
                let server_msg = self
                    .rx
                    .recv()
                    .await
                    .ok_or_else(|| anyhow::anyhow!("mem connection closed"))?;
                yield server_msg;
            }
        }
    }
}

/// Spawn a dispatch task that drives `server` from a fresh in-process
/// connection. The task lives until the returned [`MemConnection`] is dropped.
pub fn connect<S>(server: Arc<S>) -> MemConnection
where
    S: Server + Send + Sync + 'static,
{
    let (req_tx, mut req_rx) = mpsc::channel::<ClientMessage>(REPLY_CHANNEL_CAPACITY);
    let (resp_tx, resp_rx) = mpsc::channel::<ServerMessage>(REPLY_CHANNEL_CAPACITY);

    tokio::spawn(async move {
        while let Some(msg) = req_rx.recv().await {
            let stream = server.dispatch(msg);
            tokio::pin!(stream);
            while let Some(server_msg) = stream.next().await {
                if resp_tx.send(server_msg).await.is_err() {
                    return;
                }
            }
        }
    });

    MemConnection {
        tx: req_tx,
        rx: resp_rx,
    }
}
