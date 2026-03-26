//! Embedded tool dispatch loop.
//!
//! Builds a [`ToolSender`] that routes tool calls through [`RuntimeHook`]
//! without a daemon event loop. For embedded use only — the daemon uses
//! its own event-loop-based dispatch.

use crate::{RuntimeHook, bridge::RuntimeBridge};
use std::sync::Arc;
use wcore::ToolRequest;

/// Build a [`ToolSender`] that dispatches tool calls through the [`RuntimeHook`].
///
/// Spawns a background task that reads [`ToolRequest`]s and routes them
/// through `hook.dispatch_tool()`.
pub fn build_tool_sender<B: RuntimeBridge + 'static>(
    hook: Arc<RuntimeHook<B>>,
) -> wcore::ToolSender {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ToolRequest>();
    tokio::spawn(async move {
        while let Some(req) = rx.recv().await {
            let hook = hook.clone();
            tokio::spawn(async move {
                let result = hook
                    .dispatch_tool(
                        &req.name,
                        &req.args,
                        &req.agent,
                        &req.sender,
                        req.session_id,
                    )
                    .await;
                let _ = req.reply.send(result);
            });
        }
    });
    tx
}
