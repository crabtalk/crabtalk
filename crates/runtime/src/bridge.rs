//! RuntimeBridge — trait for server-specific tool dispatch.
//!
//! The runtime crate defines this trait. The daemon implements it to provide
//! `ask_user`, `delegate`, and per-session CWD resolution. Embedded users
//! get [`NoBridge`] with no-op defaults.

use std::path::PathBuf;

/// Trait for server-specific tool dispatch that the runtime cannot handle locally.
pub trait RuntimeBridge: Send + Sync {
    /// Handle `ask_user` — block until user replies.
    fn dispatch_ask_user(
        &self,
        args: &str,
        session_id: Option<u64>,
    ) -> impl std::future::Future<Output = String> + Send {
        let _ = (args, session_id);
        async { "ask_user is not available in this runtime mode".to_owned() }
    }

    /// Handle `delegate` — spawn sub-agent tasks.
    fn dispatch_delegate(
        &self,
        args: &str,
        agent: &str,
    ) -> impl std::future::Future<Output = String> + Send {
        let _ = (args, agent);
        async { "delegate is not available in this runtime mode".to_owned() }
    }

    /// Resolve the working directory for a session.
    /// Returns `None` to fall back to the runtime's base cwd.
    fn session_cwd(&self, _session_id: u64) -> Option<PathBuf> {
        None
    }
}

/// No-op bridge for embedded use.
pub struct NoBridge;

impl RuntimeBridge for NoBridge {}
