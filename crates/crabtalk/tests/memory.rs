//! Integration tests for the hook-level memory facade.

use crabtalk::hooks::Memory;
use tempfile::tempdir;
use wcore::MemoryConfig;

fn test_memory() -> Memory {
    let dir = tempdir().unwrap();
    Memory::open(MemoryConfig::default(), dir.path().join("memory.db")).unwrap()
}

#[test]
fn remember_and_recall() {
    let mem = test_memory();

    mem.remember(
        "luna-vet".to_owned(),
        "User's dog Luna has vet appointments on Thursdays. Luna is a golden retriever. Vet is Dr. Chen.".to_owned(),
        vec![],
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
        "User prefers sunny weather. Likes to go outside when sunny.".to_owned(),
        vec![],
    );
    mem.remember(
        "rust-project".to_owned(),
        "User works on a Rust project called Crabtalk. Crabtalk is an AI companion daemon written in Rust.".to_owned(),
        vec![],
    );
    mem.remember(
        "cooking".to_owned(),
        "User enjoys cooking Italian food. Favorite dish is carbonara.".to_owned(),
        vec![],
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
        "Temporary note. Should be deleted soon.".to_owned(),
        vec![],
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
fn remember_updates_existing() {
    let mem = test_memory();

    mem.remember(
        "user-pref".to_owned(),
        "User preference. Likes terse responses.".to_owned(),
        vec![],
    );
    mem.remember(
        "user-pref".to_owned(),
        "User preference updated. Likes detailed responses now.".to_owned(),
        vec![],
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
            format!("Note number {i} about testing. Content for test note {i}."),
            vec![],
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
fn aliases_boost_search() {
    let mem = test_memory();
    mem.remember(
        "deploy".to_owned(),
        "Production rollout steps and gate flipping.".to_owned(),
        vec!["ship".to_owned(), "release".to_owned()],
    );

    let result = mem.recall("ship", 5);
    assert!(result.contains("deploy"));
}
