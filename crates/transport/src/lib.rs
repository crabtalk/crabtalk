//! Transport layer for the Crabtalk daemon.
//!
//! Wire message types and API traits live in `crabtalk-proto`. This crate
//! provides the framing codec, the UDS and TCP transports, and where a
//! listener advertises itself on disk.

/// Per-connection reply channel capacity.
///
/// Bounds memory growth when a remote client consumes slowly.
/// At ~50 tokens/sec LLM streaming, 256 messages provides ~5 seconds
/// of buffer before backpressure stalls the producer.
pub const REPLY_CHANNEL_CAPACITY: usize = 256;

use anyhow::Result;
use futures_core::Stream;
use proto::{ClientMessage, ServerMessage, client::Client};
use std::{path::PathBuf, sync::LazyLock};

/// Runtime directory (`~/.crabtalk/run/`) — where a listener advertises
/// itself so a client can find it without being told.
pub static RUN_DIR: LazyLock<PathBuf> = LazyLock::new(|| crabup::dirs::CONFIG_DIR.join("run"));

/// Pinned socket path (`~/.crabtalk/run/crabtalk.sock`).
#[cfg(unix)]
pub static SOCKET_PATH: LazyLock<PathBuf> = LazyLock::new(|| RUN_DIR.join("crabtalk.sock"));

/// TCP port file (`~/.crabtalk/run/crabtalk.port`). Contains the port as text.
pub static TCP_PORT_FILE: LazyLock<PathBuf> = LazyLock::new(|| RUN_DIR.join("crabtalk.port"));

pub mod codec;
pub mod tcp;
#[cfg(unix)]
pub mod uds;

/// Transport-agnostic client connection to the crabtalk daemon.
///
/// Wraps platform-specific connection types and implements [`Client`]
/// so callers don't need to match on the transport variant.
pub enum Transport {
    #[cfg(unix)]
    Uds(uds::Connection),
    Tcp(tcp::TcpConnection),
}

/// Dispatch a method call to the inner connection regardless of variant.
macro_rules! dispatch {
    ($self:expr, |$c:ident| $body:expr) => {
        match $self {
            #[cfg(unix)]
            Transport::Uds($c) => $body,
            Transport::Tcp($c) => $body,
        }
    };
}

impl Client for Transport {
    async fn request(&mut self, msg: ClientMessage) -> Result<ServerMessage> {
        dispatch!(self, |c| c.request(msg).await)
    }

    fn request_stream(
        &mut self,
        msg: ClientMessage,
    ) -> impl Stream<Item = Result<ServerMessage>> + Send + '_ {
        async_stream::try_stream! {
            dispatch!(self, |c| {
                use futures_util::StreamExt;
                let s = c.request_stream(msg);
                tokio::pin!(s);
                while let Some(item) = s.next().await {
                    yield item?;
                }
            });
        }
    }
}
