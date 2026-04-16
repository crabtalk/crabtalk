//! Tests for Storage session persistence and sender_slug.

use crabtalk_core::{
    model::HistoryEntry,
    sender_slug,
    storage::{SessionHandle, Storage},
    testing::InMemoryStorage,
};

fn storage() -> InMemoryStorage {
    InMemoryStorage::new()
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
    let s = storage();
    let handle = s.create_session("crab", "user").unwrap();
    assert!(!handle.as_str().is_empty());
}

#[test]
fn create_persists_meta() {
    let s = storage();
    let handle = s.create_session("crab", "user").unwrap();
    let snapshot = s.load_session(&handle).unwrap().unwrap();
    assert_eq!(snapshot.meta.agent, "crab");
    assert_eq!(snapshot.meta.created_by, "user");
}

#[test]
fn append_messages_persists() {
    let s = storage();
    let handle = s.create_session("crab", "user").unwrap();
    s.append_session_messages(
        &handle,
        &[
            HistoryEntry::user("hello"),
            HistoryEntry::assistant("hi", None, None),
        ],
    )
    .unwrap();

    let snapshot = s.load_session(&handle).unwrap().unwrap();
    assert_eq!(snapshot.history.len(), 2);
}

#[test]
fn append_caller_filters_auto_injected() {
    let s = storage();
    let handle = s.create_session("crab", "user").unwrap();
    let entries = [
        HistoryEntry::user("injected").auto_injected(),
        HistoryEntry::user("real"),
    ];
    let persistable: Vec<_> = entries.into_iter().filter(|e| !e.auto_injected).collect();
    s.append_session_messages(&handle, &persistable).unwrap();

    let snapshot = s.load_session(&handle).unwrap().unwrap();
    assert_eq!(snapshot.history.len(), 1);
}

#[test]
fn load_roundtrip() {
    let s = storage();
    let handle = s.create_session("crab", "tester").unwrap();
    s.append_session_messages(
        &handle,
        &[
            HistoryEntry::user("hello"),
            HistoryEntry::assistant("world", None, None),
        ],
    )
    .unwrap();

    let snapshot = s.load_session(&handle).unwrap().unwrap();
    assert_eq!(snapshot.meta.agent, "crab");
    assert_eq!(snapshot.meta.created_by, "tester");
    assert_eq!(snapshot.history.len(), 2);
}

#[test]
fn load_after_compact() {
    let s = storage();
    let handle = s.create_session("crab", "user").unwrap();
    s.append_session_messages(
        &handle,
        &[
            HistoryEntry::user("old"),
            HistoryEntry::assistant("old reply", None, None),
        ],
    )
    .unwrap();
    // Archive marker points by name only — summary content now lives
    // in the memory db, not in session storage.
    s.append_session_compact(&handle, "archive-session-42")
        .unwrap();
    s.append_session_messages(&handle, &[HistoryEntry::user("new")])
        .unwrap();

    let snapshot = s.load_session(&handle).unwrap().unwrap();
    assert_eq!(snapshot.archive.as_deref(), Some("archive-session-42"));
    // History returned from storage no longer includes the compact
    // prefix — the caller is responsible for resolving the archive
    // name against memory.
    assert_eq!(snapshot.history.len(), 1);
    assert_eq!(snapshot.history[0].text(), "new");
}

#[test]
fn update_meta_preserves_handle() {
    let s = storage();
    let handle = s.create_session("crab", "user").unwrap();
    let mut meta = s.load_session(&handle).unwrap().unwrap().meta;
    meta.title = "My Chat".to_owned();
    s.update_session_meta(&handle, &meta).unwrap();

    let snapshot = s.load_session(&handle).unwrap().unwrap();
    assert_eq!(snapshot.meta.title, "My Chat");
}

#[test]
fn find_latest_returns_session() {
    let s = storage();
    let _h1 = s.create_session("crab", "user").unwrap();
    let h2 = s.create_session("crab", "user").unwrap();
    let found = s.find_latest_session("crab", "user").unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert!(found == h2 || found == _h1);
}

#[test]
fn load_missing_handle_returns_none() {
    let s = storage();
    let ghost = SessionHandle::new("ghost_nobody_1");
    assert!(s.load_session(&ghost).unwrap().is_none());
}

#[test]
fn find_latest_empty_repo() {
    let s = storage();
    assert!(s.find_latest_session("crab", "user").unwrap().is_none());
}

#[test]
fn find_latest_no_match() {
    let s = storage();
    s.create_session("other", "user").unwrap();
    assert!(s.find_latest_session("crab", "user").unwrap().is_none());
}

#[test]
fn create_assigns_distinct_handles() {
    let s = storage();
    let h1 = s.create_session("crab", "user").unwrap();
    let h2 = s.create_session("crab", "user").unwrap();
    assert_ne!(h1, h2);
}
