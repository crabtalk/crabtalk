//! Walrus gateway — application shell composing runtime, channels,
//! sessions, authentication, and cron scheduling.

use runtime::Hook;
use std::sync::Arc;

pub mod api_key;
pub mod auth;
pub mod backend;
pub mod config;
pub mod cron;
pub mod router;
pub mod session;
pub mod state;
pub mod utils;
pub mod ws;

pub use api_key::ApiKeyAuthenticator;
pub use auth::{AuthContext, AuthError, Authenticator};
pub use backend::MemoryBackend;
pub use config::GatewayConfig;
pub use cron::{CronJob, CronScheduler};
pub use router::{ChannelRouter, RoutingRule};
pub use session::{Session, SessionManager, SessionScope, TrustLevel};
pub use state::AppState;

/// The gateway application shell.
///
/// Holds a runtime and configuration. Generic over `H: Hook` to support
/// different memory backends. Monomorphized with a concrete hook in the
/// binary entry point.
pub struct Gateway<H: Hook + 'static> {
    /// Gateway configuration loaded from TOML.
    pub config: GatewayConfig,
    /// The walrus runtime, shared across handlers.
    pub runtime: Arc<runtime::Runtime<H>>,
}

impl<H: Hook + 'static> Gateway<H> {
    /// Create a new gateway from configuration and a pre-built runtime.
    pub fn new(config: GatewayConfig, runtime: runtime::Runtime<H>) -> Self {
        Self {
            config,
            runtime: Arc::new(runtime),
        }
    }
}
