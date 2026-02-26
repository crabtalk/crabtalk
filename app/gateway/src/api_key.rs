//! API key authenticator implementation.
//!
//! Simple lookup-based authentication mapping API keys to trust levels.

use crate::{
    TrustLevel,
    auth::{AuthContext, AuthError, Authenticator},
    config::AuthConfig,
};
use compact_str::CompactString;
use std::collections::BTreeMap;

/// Authenticates clients via static API key lookup.
pub struct ApiKeyAuthenticator {
    /// Map from API key to trust level.
    keys: BTreeMap<CompactString, TrustLevel>,
}

impl ApiKeyAuthenticator {
    /// Create from a map of key -> trust level pairs.
    pub fn new(keys: BTreeMap<CompactString, TrustLevel>) -> Self {
        Self { keys }
    }

    /// Create from [`AuthConfig`].
    ///
    /// All keys from config are granted `TrustLevel::Trusted`.
    pub fn from_config(config: &AuthConfig) -> Self {
        let keys = config
            .api_keys
            .iter()
            .map(|k| (CompactString::new(k), TrustLevel::Trusted))
            .collect();
        Self { keys }
    }
}

impl Authenticator for ApiKeyAuthenticator {
    fn authenticate(
        &self,
        token: &str,
    ) -> impl std::future::Future<Output = Result<AuthContext, AuthError>> + Send {
        let result = self
            .keys
            .get(token)
            .map(|&trust_level| AuthContext {
                identity: CompactString::new(token),
                trust_level,
            })
            .ok_or(AuthError::InvalidToken);
        std::future::ready(result)
    }
}
