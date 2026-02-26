//! Session management tests.

use walrus_gateway::{SessionManager, SessionScope, TrustLevel};

#[test]
fn create_and_get_session() {
    let mgr = SessionManager::new();
    let session = mgr.create(SessionScope::Main, TrustLevel::Trusted);
    assert!(!session.id.is_empty());
    assert_eq!(session.scope, SessionScope::Main);
    assert_eq!(session.trust_level, TrustLevel::Trusted);

    let retrieved = mgr.get(&session.id).unwrap();
    assert_eq!(retrieved.id, session.id);
}

#[test]
fn remove_session() {
    let mgr = SessionManager::new();
    let session = mgr.create(SessionScope::Main, TrustLevel::Admin);
    assert_eq!(mgr.len(), 1);

    let removed = mgr.remove(&session.id);
    assert!(removed.is_some());
    assert!(mgr.get(&session.id).is_none());
    assert!(mgr.is_empty());
}

#[test]
fn touch_updates_last_active() {
    let mgr = SessionManager::new();
    let session = mgr.create(SessionScope::Main, TrustLevel::Trusted);
    let original = session.last_active;

    // Sleep briefly to ensure timestamp changes
    std::thread::sleep(std::time::Duration::from_millis(1100));
    mgr.touch(&session.id);

    let updated = mgr.get(&session.id).unwrap();
    assert!(updated.last_active >= original);
}

#[test]
fn cleanup_expired() {
    let mgr = SessionManager::new();
    let _s1 = mgr.create(SessionScope::Main, TrustLevel::Trusted);
    let _s2 = mgr.create(SessionScope::Dm("peer-1".into()), TrustLevel::Untrusted);
    assert_eq!(mgr.len(), 2);

    // Sleep so sessions are in the past, then cleanup with 0 max age
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let removed = mgr.cleanup_expired(0);
    assert_eq!(removed, 2);
    assert!(mgr.is_empty());
}

#[test]
fn session_scope_variants() {
    let mgr = SessionManager::new();

    let main = mgr.create(SessionScope::Main, TrustLevel::Admin);
    assert_eq!(main.scope, SessionScope::Main);

    let dm = mgr.create(SessionScope::Dm("user-123".into()), TrustLevel::Trusted);
    assert_eq!(dm.scope, SessionScope::Dm("user-123".into()));

    let group = mgr.create(
        SessionScope::Group("group-456".into()),
        TrustLevel::Untrusted,
    );
    assert_eq!(group.scope, SessionScope::Group("group-456".into()));

    let cron = mgr.create(SessionScope::Cron("daily-job".into()), TrustLevel::Admin);
    assert_eq!(cron.scope, SessionScope::Cron("daily-job".into()));
}

#[test]
fn trust_level_ordering() {
    assert!(TrustLevel::Untrusted < TrustLevel::Trusted);
    assert!(TrustLevel::Trusted < TrustLevel::Admin);
}

#[test]
fn default_session_manager() {
    let mgr = SessionManager::default();
    assert!(mgr.is_empty());
}
