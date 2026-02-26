//! Shared application state for the gateway server.

use crate::auth::Authenticator;
use crate::session::SessionManager;
use runtime::{Hook, Runtime};
use std::sync::Arc;

/// Shared state available to all request handlers.
pub struct AppState<H: Hook + 'static, A: Authenticator> {
    /// The walrus runtime (immutable after init).
    pub runtime: Arc<Runtime<H>>,
    /// Session manager.
    pub sessions: Arc<SessionManager>,
    /// Authenticator.
    pub authenticator: Arc<A>,
}

impl<H: Hook + 'static, A: Authenticator> Clone for AppState<H, A> {
    fn clone(&self) -> Self {
        Self {
            runtime: Arc::clone(&self.runtime),
            sessions: Arc::clone(&self.sessions),
            authenticator: Arc::clone(&self.authenticator),
        }
    }
}
