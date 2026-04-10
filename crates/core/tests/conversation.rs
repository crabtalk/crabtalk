//! Tests for SessionRepo persistence and sender_slug.

use crabtalk_core::{
    model::HistoryEntry,
    repos::{SessionRepo, mem::InMemorySessionRepo},
    sender_slug,
};

fn repo() -> InMemorySessionRepo {
    InMemorySessionRepo::new()
}

#[test]
fn sender_slug_basic() {
    assert_eq!(sender_slug("hello"), "hello");
}

#[test]
fn sender_slug_special_chars() {
    assert_eq!(sender_slug("TG:user-123"), "tg-user-123");
}

#[test]
fn sender_slug_collapses_dashes() {
    assert_eq!(sender_slug("a::b"), "a-b");
}

#[test]
fn sender_slug_empty() {
    assert_eq!(sender_slug(""), "");
}

#[test]
fn sender_slug_all_special() {
    assert_eq!(sender_slug(":::"), "");
}

#[test]
fn create_returns_handle() {
    let repo = repo();
    let handle = repo.create("crab", "user").unwrap();
    assert!(!handle.as_str().is_empty());
}

#[test]
fn create_persists_meta() {
    let repo = repo();
    let handle = repo.create("crab", "user").unwrap();
    let snapshot = repo.load(&handle).unwrap().unwrap();
    assert_eq!(snapshot.meta.agent, "crab");
    assert_eq!(snapshot.meta.created_by, "user");
}

#[test]
fn append_messages_persists() {
    let repo = repo();
    let handle = repo.create("crab", "user").unwrap();
    repo.append_messages(
        &handle,
        &[
            HistoryEntry::user("hello"),
            HistoryEntry::assistant("hi", None, None),
        ],
    )
    .unwrap();

    let snapshot = repo.load(&handle).unwrap().unwrap();
    assert_eq!(snapshot.history.len(), 2);
}

#[test]
fn append_caller_filters_auto_injected() {
    let repo = repo();
    let handle = repo.create("crab", "user").unwrap();
    let entries = [
        HistoryEntry::user("injected").auto_injected(),
        HistoryEntry::user("real"),
    ];
    // Caller is responsible for filtering auto-injected entries.
    let persistable: Vec<_> = entries.into_iter().filter(|e| !e.auto_injected).collect();
    repo.append_messages(&handle, &persistable).unwrap();

    let snapshot = repo.load(&handle).unwrap().unwrap();
    assert_eq!(snapshot.history.len(), 1);
}

#[test]
fn load_roundtrip() {
    let repo = repo();
    let handle = repo.create("crab", "tester").unwrap();
    repo.append_messages(
        &handle,
        &[
            HistoryEntry::user("hello"),
            HistoryEntry::assistant("world", None, None),
        ],
    )
    .unwrap();

    let snapshot = repo.load(&handle).unwrap().unwrap();
    assert_eq!(snapshot.meta.agent, "crab");
    assert_eq!(snapshot.meta.created_by, "tester");
    assert_eq!(snapshot.history.len(), 2);
}

#[test]
fn load_after_compact() {
    let repo = repo();
    let handle = repo.create("crab", "user").unwrap();
    repo.append_messages(
        &handle,
        &[
            HistoryEntry::user("old"),
            HistoryEntry::assistant("old reply", None, None),
        ],
    )
    .unwrap();
    repo.append_compact(&handle, "summary of conversation")
        .unwrap();
    repo.append_messages(&handle, &[HistoryEntry::user("new")])
        .unwrap();

    let snapshot = repo.load(&handle).unwrap().unwrap();
    // After compact: summary-as-user-message + new message
    assert_eq!(snapshot.history.len(), 2);
    assert_eq!(snapshot.history[0].text(), "summary of conversation");
}

#[test]
fn update_meta_preserves_handle() {
    let repo = repo();
    let handle = repo.create("crab", "user").unwrap();
    let mut meta = repo.load(&handle).unwrap().unwrap().meta;
    meta.title = "My Chat".to_owned();
    repo.update_meta(&handle, &meta).unwrap();

    // Handle is stable — title change doesn't change identity.
    let snapshot = repo.load(&handle).unwrap().unwrap();
    assert_eq!(snapshot.meta.title, "My Chat");
}

#[test]
fn find_latest_returns_session() {
    let repo = repo();
    let _h1 = repo.create("crab", "user").unwrap();
    let h2 = repo.create("crab", "user").unwrap();
    let found = repo.find_latest("crab", "user").unwrap();
    // Should find one of them (implementation-defined which one for
    // HashMap-based storage, but at least it finds something).
    assert!(found.is_some());
    let found = found.unwrap();
    assert!(found == h2 || found == _h1);
}

#[test]
fn load_missing_handle_returns_none() {
    use crabtalk_core::repos::SessionHandle;

    let repo = repo();
    let ghost = SessionHandle::new("ghost_nobody_1");
    assert!(repo.load(&ghost).unwrap().is_none());
}

#[test]
fn find_latest_empty_repo() {
    let repo = repo();
    assert!(repo.find_latest("crab", "user").unwrap().is_none());
}

#[test]
fn find_latest_no_match() {
    let repo = repo();
    repo.create("other", "user").unwrap();
    assert!(repo.find_latest("crab", "user").unwrap().is_none());
}

#[test]
fn create_assigns_distinct_handles() {
    let repo = repo();
    let h1 = repo.create("crab", "user").unwrap();
    let h2 = repo.create("crab", "user").unwrap();
    assert_ne!(h1, h2);
}
