//! Integration tests for the memory system using InMemoryMemoryRepo (no disk I/O).

use runtime::{Memory, MemoryConfig};
use std::sync::Arc;
use wcore::repos::{MemoryRepo, mem::InMemoryMemoryRepo};

fn test_memory() -> Memory<InMemoryMemoryRepo> {
    Memory::open(MemoryConfig::default(), Arc::new(InMemoryMemoryRepo::new()))
}

#[test]
fn remember_and_recall() {
    let mem = test_memory();

    mem.remember(
        "luna-vet".to_owned(),
        "User's dog Luna has vet appointments on Thursdays".to_owned(),
        "Luna is a golden retriever. Vet is Dr. Chen.".to_owned(),
    );

    let result = mem.recall("luna vet", 5);
    assert!(result.contains("luna-vet"), "should find luna-vet entry");
    assert!(result.contains("Dr. Chen"), "should contain entry content");
}

#[test]
fn recall_ranks_by_relevance() {
    let mem = test_memory();

    mem.remember(
        "weather".to_owned(),
        "User prefers sunny weather".to_owned(),
        "Likes to go outside when sunny.".to_owned(),
    );
    mem.remember(
        "rust-project".to_owned(),
        "User works on a Rust project called Crabtalk".to_owned(),
        "Crabtalk is an AI companion daemon written in Rust.".to_owned(),
    );
    mem.remember(
        "cooking".to_owned(),
        "User enjoys cooking Italian food".to_owned(),
        "Favorite dish is carbonara.".to_owned(),
    );

    let result = mem.recall("rust crabtalk", 5);
    assert!(
        result.starts_with("## rust-project"),
        "rust-project should rank first, got: {result}"
    );
}

#[test]
fn forget_removes_entry() {
    let mem = test_memory();

    mem.remember(
        "temp-note".to_owned(),
        "Temporary note".to_owned(),
        "Should be deleted.".to_owned(),
    );

    let result = mem.recall("temporary", 5);
    assert!(result.contains("temp-note"));

    let result = mem.forget("temp-note");
    assert!(result.contains("forgot"));

    let result = mem.recall("temporary", 5);
    assert_eq!(result, "no memories found");
}

#[test]
fn forget_nonexistent_returns_error() {
    let mem = test_memory();
    let result = mem.forget("does-not-exist");
    assert!(result.contains("no entry named"));
}

#[test]
fn write_index_and_build_prompt() {
    let mem = test_memory();

    mem.write_index("# My Overview\n\nI know about Luna the dog.");

    let prompt = mem.build_prompt();
    assert!(prompt.contains("<memory>"));
    assert!(prompt.contains("Luna the dog"));
    assert!(prompt.contains("</memory>"));
}

#[test]
fn build_prompt_empty_index() {
    let mem = test_memory();
    let prompt = mem.build_prompt();
    assert!(!prompt.contains("<memory>\n\n</memory>"));
}

#[test]
fn remember_updates_existing() {
    let mem = test_memory();

    mem.remember(
        "user-pref".to_owned(),
        "User preference".to_owned(),
        "Likes terse responses.".to_owned(),
    );
    mem.remember(
        "user-pref".to_owned(),
        "User preference updated".to_owned(),
        "Likes detailed responses now.".to_owned(),
    );

    let result = mem.recall("preference", 5);
    assert!(result.contains("detailed responses"));
    assert!(!result.contains("terse responses"));
}

#[test]
fn recall_empty_memory() {
    let mem = test_memory();
    let result = mem.recall("anything", 5);
    assert_eq!(result, "no memories found");
}

#[test]
fn recall_respects_limit() {
    let mem = test_memory();

    for i in 0..10 {
        mem.remember(
            format!("note-{i}"),
            format!("Note number {i} about testing"),
            format!("Content for test note {i}."),
        );
    }

    let result = mem.recall("testing note", 3);
    let entries: Vec<&str> = result.split("\n---\n").collect();
    assert!(
        entries.len() <= 3,
        "should return at most 3 entries, got {}",
        entries.len()
    );
}

#[test]
fn migration_converts_legacy_files() {
    use wcore::repos::MemoryEntry;

    let repo = Arc::new(InMemoryMemoryRepo::new());

    // Pre-populate the repo with entries that would have come from legacy files.
    repo.save(&MemoryEntry {
        name: "memory-md".to_owned(),
        description: "Legacy memory.md content".to_owned(),
        content: "Luna is a golden retriever\n\nUser works on Crabtalk".to_owned(),
    })
    .unwrap();
    repo.save(&MemoryEntry {
        name: "user-md".to_owned(),
        description: "Legacy user.md content".to_owned(),
        content: "Name: Alice\nRole: Developer".to_owned(),
    })
    .unwrap();
    repo.save(&MemoryEntry {
        name: "facts-toml".to_owned(),
        description: "Legacy facts.toml content".to_owned(),
        content: "dog_name = \"Luna\"\nlanguage = \"Rust\"".to_owned(),
    })
    .unwrap();

    let mem = Memory::open(MemoryConfig::default(), repo);

    let result = mem.recall("golden retriever", 5);
    assert!(result.contains("golden retriever"));

    let result = mem.recall("Alice", 5);
    assert!(result.contains("Alice"));

    let result = mem.recall("Luna", 5);
    assert!(result.contains("Luna"));
}

#[test]
fn slugify_examples() {
    use runtime::memory::entry::slugify;

    assert_eq!(slugify("Luna's Vet Appointment!"), "luna-s-vet-appointment");
    assert_eq!(slugify("hello world"), "hello-world");
    assert_eq!(slugify("---dashes---"), "dashes");
    assert_eq!(slugify("CamelCase"), "camelcase");
    assert_eq!(slugify(""), "entry");
    assert_eq!(slugify("!!!"), "entry");
}

#[test]
fn entry_parse_roundtrip() {
    use runtime::memory::entry::MemoryEntry;

    let entry = MemoryEntry {
        name: "test-entry".to_owned(),
        description: "A test entry for round-trip".to_owned(),
        content: "Some content here.".to_owned(),
    };

    let serialized = entry.serialize();
    let parsed = MemoryEntry::parse(&serialized).unwrap();

    assert_eq!(parsed.name, "test-entry");
    assert_eq!(parsed.description, "A test entry for round-trip");
    assert_eq!(parsed.content, "Some content here.");
}

#[test]
fn bm25_tokenize() {
    use runtime::memory::bm25::tokenize;

    let tokens = tokenize("Hello, World! This is a test.");
    assert!(tokens.contains(&"hello".to_owned()));
    assert!(tokens.contains(&"world".to_owned()));
    assert!(tokens.contains(&"test".to_owned()));
    assert!(!tokens.contains(&"this".to_owned()));
    assert!(!tokens.contains(&"is".to_owned()));
    assert!(!tokens.contains(&"a".to_owned()));
}

#[test]
fn bm25_score_ranks() {
    use runtime::memory::bm25::score;

    let docs = vec![
        (0, "the weather is sunny today"),
        (1, "rust programming language systems"),
        (2, "rust compiler and cargo build tool"),
    ];

    let results = score(&docs, "rust programming", 5);
    assert!(!results.is_empty());
    assert_eq!(results[0].0, 1);
}
