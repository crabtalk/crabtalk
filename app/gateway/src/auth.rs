//! Authentication interface for the gateway.
//!
//! Defines the `Authenticator` trait and `AuthContext` for verifying
//! client credentials. Concrete implementations live in separate files.

use compact_str::CompactString;
use std::future::Future;

/// Authentication context returned on successful authentication.
#[derive(Debug, Clone)]
pub struct AuthContext {
    /// Identifier for the authenticated entity (e.g. key name).
    pub identity: CompactString,
    /// Trust level granted to this entity.
    pub trust_level: crate::TrustLevel,
}

/// Authentication error.
#[derive(Debug, Clone)]
pub enum AuthError {
    /// The provided token is invalid or unknown.
    InvalidToken,
    /// The token has expired.
    Expired,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidToken => write!(f, "invalid or unknown token"),
            Self::Expired => write!(f, "token expired"),
        }
    }
}

impl std::error::Error for AuthError {}

/// Trait for authenticating client connections.
///
/// Uses RPITIT (no dyn dispatch) per DD#11.
pub trait Authenticator: Send + Sync {
    /// Verify a token and return the authentication context.
    fn authenticate(
        &self,
        token: &str,
    ) -> impl Future<Output = Result<AuthContext, AuthError>> + Send;
}
