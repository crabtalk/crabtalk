//! Node event types and dispatch.
//!
//! Only `Shutdown` remains. The event loop will go away entirely in
//! the next phase — it no longer coordinates any work.

use crate::node::Node;
use crabllm_core::Provider;
use runtime::host::Host;
use tokio::sync::mpsc;

/// Inbound event from any source, processed by the central event loop.
pub enum NodeEvent {
    /// Graceful shutdown request.
    Shutdown,
}

/// Shorthand for the event sender half of the daemon event channel.
pub type NodeEventSender = mpsc::UnboundedSender<NodeEvent>;

// ── Event dispatch ───────────────────────────────────────────────────

impl<P: Provider + 'static, H: Host + 'static> Node<P, H> {
    /// Drain the legacy event loop until `Shutdown` is received.
    pub(crate) async fn handle_events(&self, mut rx: mpsc::UnboundedReceiver<NodeEvent>) {
        tracing::info!("event loop started");
        while let Some(event) = rx.recv().await {
            match event {
                NodeEvent::Shutdown => {
                    tracing::info!("event loop shutting down");
                    break;
                }
            }
        }
        tracing::info!("event loop stopped");
    }
}
