//! Tests for Conversation JSONL persistence and sender_slug.

use crabtalk_core::{Conversation, find_latest_conversation, model::Message, sender_slug};
use std::io::Write;
use tempfile::TempDir;

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
fn init_file_creates_jsonl() {
    let dir = TempDir::new().unwrap();
    let mut conversation = Conversation::new(1, "crab", "user");
    conversation.init_file(dir.path());
    assert!(conversation.file_path.is_some());
    let path = conversation.file_path.as_ref().unwrap();
    assert!(path.exists());
    assert!(path.to_str().unwrap().ends_with(".jsonl"));
}

#[test]
fn init_file_meta_line() {
    let dir = TempDir::new().unwrap();
    let mut conversation = Conversation::new(1, "crab", "user");
    conversation.init_file(dir.path());
    let content = std::fs::read_to_string(conversation.file_path.as_ref().unwrap()).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 1);
    assert!(content.contains("\"agent\":\"crab\""));
    assert!(content.contains("\"created_by\":\"user\""));
}

#[test]
fn append_messages_persists() {
    let dir = TempDir::new().unwrap();
    let mut conversation = Conversation::new(1, "crab", "user");
    conversation.init_file(dir.path());
    conversation.append_messages(&[Message::user("hello"), Message::assistant("hi", None, None)]);
    let content = std::fs::read_to_string(conversation.file_path.as_ref().unwrap()).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 3); // meta + 2 messages
}

#[test]
fn append_skips_auto_injected() {
    let dir = TempDir::new().unwrap();
    let mut conversation = Conversation::new(1, "crab", "user");
    conversation.init_file(dir.path());
    let mut msg = Message::user("injected");
    msg.auto_injected = true;
    conversation.append_messages(&[msg, Message::user("real")]);
    let content = std::fs::read_to_string(conversation.file_path.as_ref().unwrap()).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2); // meta + 1 real message
}

#[test]
fn load_context_roundtrip() {
    let dir = TempDir::new().unwrap();
    let mut conversation = Conversation::new(1, "crab", "tester");
    conversation.init_file(dir.path());
    conversation.append_messages(&[
        Message::user("hello"),
        Message::assistant("world", None, None),
    ]);

    let (meta, messages) =
        Conversation::load_context(conversation.file_path.as_ref().unwrap()).unwrap();
    assert_eq!(meta.agent, "crab");
    assert_eq!(meta.created_by, "tester");
    assert_eq!(messages.len(), 2);
}

#[test]
fn load_context_after_compact() {
    let dir = TempDir::new().unwrap();
    let mut conversation = Conversation::new(1, "crab", "user");
    conversation.init_file(dir.path());
    conversation.append_messages(&[
        Message::user("old"),
        Message::assistant("old reply", None, None),
    ]);
    conversation.append_compact("summary of conversation");
    conversation.append_messages(&[Message::user("new")]);

    let (_, messages) =
        Conversation::load_context(conversation.file_path.as_ref().unwrap()).unwrap();
    // After compact: summary-as-user-message + new message
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "summary of conversation");
}

#[test]
fn set_title_renames_file() {
    let dir = TempDir::new().unwrap();
    let mut conversation = Conversation::new(1, "crab", "user");
    conversation.init_file(dir.path());
    let old_path = conversation.file_path.clone().unwrap();
    conversation.set_title("My Chat");
    assert_ne!(conversation.file_path.as_ref().unwrap(), &old_path);
    assert!(
        conversation
            .file_path
            .as_ref()
            .unwrap()
            .to_str()
            .unwrap()
            .contains("my-chat")
    );
}

#[test]
fn find_latest_conversation_picks_highest_seq() {
    let dir = TempDir::new().unwrap();

    // Create conversation files manually with different seq numbers
    for seq in [1, 3, 2] {
        let name = format!("crab_user_{seq}.jsonl");
        let path = dir.path().join(&name);
        let mut f = std::fs::File::create(&path).unwrap();
        let meta = format!(
            r#"{{"agent":"crab","created_by":"user","created_at":"2024-01-01T00:00:00Z"}}"#
        );
        writeln!(f, "{meta}").unwrap();
    }

    let found = find_latest_conversation(dir.path(), "crab", "user").unwrap();
    assert!(found.to_str().unwrap().contains("crab_user_3"));
}

// --- Error paths ---

#[test]
fn load_context_empty_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("empty.jsonl");
    std::fs::write(&path, "").unwrap();
    assert!(Conversation::load_context(&path).is_err());
}

#[test]
fn load_context_invalid_meta() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("bad_meta.jsonl");
    std::fs::write(&path, "not valid json\n").unwrap();
    assert!(Conversation::load_context(&path).is_err());
}

#[test]
fn load_context_skips_invalid_message_lines() {
    let dir = TempDir::new().unwrap();
    let mut conversation = Conversation::new(1, "crab", "user");
    conversation.init_file(dir.path());
    conversation.append_messages(&[Message::user("valid")]);
    // Manually append a corrupt line
    let path = conversation.file_path.as_ref().unwrap();
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(f, "{{this is not valid json}}").unwrap();
    writeln!(f, "").unwrap(); // empty line

    let (_, messages) = Conversation::load_context(path).unwrap();
    assert_eq!(messages.len(), 1); // only the valid message
}

#[test]
fn load_context_nonexistent_file() {
    let result = Conversation::load_context(std::path::Path::new("/nonexistent/path.jsonl"));
    assert!(result.is_err());
}

#[test]
fn find_latest_conversation_empty_dir() {
    let dir = TempDir::new().unwrap();
    assert!(find_latest_conversation(dir.path(), "crab", "user").is_none());
}

#[test]
fn find_latest_conversation_no_match() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("other_agent_user_1.jsonl");
    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(
        f,
        r#"{{"agent":"other","created_by":"user","created_at":"2024-01-01T00:00:00Z"}}"#
    )
    .unwrap();
    assert!(find_latest_conversation(dir.path(), "crab", "user").is_none());
}

#[test]
fn seq_increments_across_init_file_calls() {
    let dir = TempDir::new().unwrap();
    let mut c1 = Conversation::new(1, "crab", "user");
    c1.init_file(dir.path());
    let mut c2 = Conversation::new(2, "crab", "user");
    c2.init_file(dir.path());
    // c2 should have a higher seq number
    let p1 = c1
        .file_path
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let p2 = c2
        .file_path
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_ne!(p1, p2);
}
