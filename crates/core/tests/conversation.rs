//! Tests for Conversation Storage persistence and sender_slug.

use crabtalk_core::{
    Conversation, MemStorage, Storage, find_latest_conversation, model::HistoryEntry, sender_slug,
};
use std::sync::Arc;

fn mem() -> Arc<dyn Storage> {
    Arc::new(MemStorage::new())
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
fn ensure_slug_creates_meta() {
    let storage = mem();
    let mut conversation = Conversation::new(1, "crab", "user");
    conversation.ensure_slug(storage.as_ref());
    assert!(conversation.slug.is_some());
    let meta_key = format!("sessions/{}/meta", conversation.slug.as_ref().unwrap());
    assert!(storage.get(&meta_key).unwrap().is_some());
}

#[test]
fn ensure_slug_meta_content() {
    let storage = mem();
    let mut conversation = Conversation::new(1, "crab", "user");
    conversation.ensure_slug(storage.as_ref());
    let slug = conversation.slug.as_ref().unwrap();
    let meta_bytes = storage
        .get(&format!("sessions/{slug}/meta"))
        .unwrap()
        .unwrap();
    let meta_str = std::str::from_utf8(&meta_bytes).unwrap();
    assert!(meta_str.contains("\"agent\":\"crab\""));
    assert!(meta_str.contains("\"created_by\":\"user\""));
}

#[test]
fn append_messages_persists() {
    let storage = mem();
    let mut conversation = Conversation::new(1, "crab", "user");
    conversation.ensure_slug(storage.as_ref());
    conversation.append_messages(
        storage.as_ref(),
        &[
            HistoryEntry::user("hello"),
            HistoryEntry::assistant("hi", None, None),
        ],
    );
    let slug = conversation.slug.clone().unwrap();
    let step_keys = storage.list(&format!("sessions/{slug}/step-")).unwrap();
    assert_eq!(step_keys.len(), 2);
}

#[test]
fn append_skips_auto_injected() {
    let storage = mem();
    let mut conversation = Conversation::new(1, "crab", "user");
    conversation.ensure_slug(storage.as_ref());
    let injected = HistoryEntry::user("injected").auto_injected();
    conversation.append_messages(storage.as_ref(), &[injected, HistoryEntry::user("real")]);
    let slug = conversation.slug.clone().unwrap();
    let step_keys = storage.list(&format!("sessions/{slug}/step-")).unwrap();
    assert_eq!(step_keys.len(), 1);
}

#[test]
fn load_context_roundtrip() {
    let storage = mem();
    let mut conversation = Conversation::new(1, "crab", "tester");
    conversation.ensure_slug(storage.as_ref());
    conversation.append_messages(
        storage.as_ref(),
        &[
            HistoryEntry::user("hello"),
            HistoryEntry::assistant("world", None, None),
        ],
    );

    let slug = conversation.slug.clone().unwrap();
    let (meta, entries) = Conversation::load_context(storage.as_ref(), &slug).unwrap();
    assert_eq!(meta.agent, "crab");
    assert_eq!(meta.created_by, "tester");
    assert_eq!(entries.len(), 2);
}

#[test]
fn load_context_after_compact() {
    let storage = mem();
    let mut conversation = Conversation::new(1, "crab", "user");
    conversation.ensure_slug(storage.as_ref());
    conversation.append_messages(
        storage.as_ref(),
        &[
            HistoryEntry::user("old"),
            HistoryEntry::assistant("old reply", None, None),
        ],
    );
    conversation.append_compact(storage.as_ref(), "summary of conversation");
    conversation.append_messages(storage.as_ref(), &[HistoryEntry::user("new")]);

    let slug = conversation.slug.clone().unwrap();
    let (_, entries) = Conversation::load_context(storage.as_ref(), &slug).unwrap();
    // After compact: summary-as-user-message + new message
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].text(), "summary of conversation");
}

#[test]
fn set_title_updates_meta_without_rename() {
    let storage = mem();
    let mut conversation = Conversation::new(1, "crab", "user");
    conversation.ensure_slug(storage.as_ref());
    let slug = conversation.slug.clone().unwrap();
    conversation.set_title(storage.as_ref(), "My Chat");
    // Slug is stable — rename is now a metadata edit, not a key move.
    assert_eq!(conversation.slug.as_ref().unwrap(), &slug);
    let (meta, _) = Conversation::load_context(storage.as_ref(), &slug).unwrap();
    assert_eq!(meta.title, "My Chat");
}

#[test]
fn find_latest_conversation_picks_highest_seq() {
    let storage = mem();
    // Seed three slugs manually.
    for seq in [1u32, 3, 2] {
        let slug = format!("crab_user_{seq}");
        storage
            .put(
                &format!("sessions/{slug}/meta"),
                br#"{"agent":"crab","created_by":"user","created_at":"2024-01-01T00:00:00Z"}"#,
            )
            .unwrap();
    }
    let found = find_latest_conversation(storage.as_ref(), "crab", "user").unwrap();
    assert_eq!(found, "crab_user_3");
}

#[test]
fn load_context_missing_slug_errors() {
    let storage = mem();
    assert!(Conversation::load_context(storage.as_ref(), "ghost_nobody_1").is_err());
}

#[test]
fn load_context_skips_invalid_steps() {
    let storage = mem();
    let mut conversation = Conversation::new(1, "crab", "user");
    conversation.ensure_slug(storage.as_ref());
    conversation.append_messages(storage.as_ref(), &[HistoryEntry::user("valid")]);
    // Manually inject a corrupt step between meta and the real step.
    let slug = conversation.slug.clone().unwrap();
    storage
        .put(&format!("sessions/{slug}/step-999999"), b"{{garbage}")
        .unwrap();

    let (_, entries) = Conversation::load_context(storage.as_ref(), &slug).unwrap();
    assert_eq!(entries.len(), 1);
}

#[test]
fn find_latest_conversation_empty_storage() {
    let storage = mem();
    assert!(find_latest_conversation(storage.as_ref(), "crab", "user").is_none());
}

#[test]
fn find_latest_conversation_no_match() {
    let storage = mem();
    storage
        .put(
            "sessions/other_user_1/meta",
            br#"{"agent":"other","created_by":"user","created_at":"2024-01-01T00:00:00Z"}"#,
        )
        .unwrap();
    assert!(find_latest_conversation(storage.as_ref(), "crab", "user").is_none());
}

#[test]
fn seq_increments_across_ensure_slug_calls() {
    let storage = mem();
    let mut c1 = Conversation::new(1, "crab", "user");
    c1.ensure_slug(storage.as_ref());
    let mut c2 = Conversation::new(2, "crab", "user");
    c2.ensure_slug(storage.as_ref());
    assert_ne!(c1.slug, c2.slug);
    // c2's slug should carry the higher sequence.
    let c1_slug = c1.slug.unwrap();
    let c2_slug = c2.slug.unwrap();
    let c1_seq: u32 = c1_slug.rsplit('_').next().unwrap().parse().unwrap();
    let c2_seq: u32 = c2_slug.rsplit('_').next().unwrap().parse().unwrap();
    assert!(c2_seq > c1_seq);
}
