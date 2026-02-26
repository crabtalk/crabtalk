//! Authentication tests.

use walrus_gateway::{
    ApiKeyAuthenticator, AuthError, Authenticator, TrustLevel, config::AuthConfig,
};

#[tokio::test]
async fn valid_key_authenticates() {
    let config = AuthConfig {
        api_keys: vec!["test-key-1".to_string(), "test-key-2".to_string()],
    };
    let auth = ApiKeyAuthenticator::from_config(&config);

    let ctx = auth.authenticate("test-key-1").await.unwrap();
    assert_eq!(ctx.identity.as_str(), "test-key-1");
    assert_eq!(ctx.trust_level, TrustLevel::Trusted);
}

#[tokio::test]
async fn invalid_key_rejected() {
    let config = AuthConfig {
        api_keys: vec!["valid-key".to_string()],
    };
    let auth = ApiKeyAuthenticator::from_config(&config);

    let err = auth.authenticate("wrong-key").await.unwrap_err();
    assert!(matches!(err, AuthError::InvalidToken));
}

#[tokio::test]
async fn empty_token_rejected() {
    let config = AuthConfig {
        api_keys: vec!["valid-key".to_string()],
    };
    let auth = ApiKeyAuthenticator::from_config(&config);

    let err = auth.authenticate("").await.unwrap_err();
    assert!(matches!(err, AuthError::InvalidToken));
}

#[tokio::test]
async fn custom_trust_levels() {
    use compact_str::CompactString;
    use std::collections::BTreeMap;

    let mut keys = BTreeMap::new();
    keys.insert(CompactString::new("admin-key"), TrustLevel::Admin);
    keys.insert(CompactString::new("user-key"), TrustLevel::Trusted);

    let auth = ApiKeyAuthenticator::new(keys);

    let admin = auth.authenticate("admin-key").await.unwrap();
    assert_eq!(admin.trust_level, TrustLevel::Admin);

    let user = auth.authenticate("user-key").await.unwrap();
    assert_eq!(user.trust_level, TrustLevel::Trusted);
}

#[tokio::test]
async fn empty_config_rejects_all() {
    let config = AuthConfig::default();
    let auth = ApiKeyAuthenticator::from_config(&config);

    let err = auth.authenticate("any-key").await.unwrap_err();
    assert!(matches!(err, AuthError::InvalidToken));
}

#[test]
fn auth_error_display() {
    assert_eq!(
        AuthError::InvalidToken.to_string(),
        "invalid or unknown token"
    );
    assert_eq!(AuthError::Expired.to_string(), "token expired");
}
