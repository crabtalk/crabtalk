//! Integration tests for the hook-level memory facade.

use crabtalk::hooks::Memory;
use std::fs;
use tempfile::tempdir;
use wcore::MemoryConfig;

fn test_memory() -> Memory {
    let dir = tempdir().unwrap();
    Memory::open(MemoryConfig::default(), dir.path().join("memory.db"), None).unwrap()
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
fn write_prompt_and_build_prompt() {
    let mem = test_memory();

    mem.write_prompt("# My Overview\n\nI know about Luna the dog.");

    let prompt = mem.build_prompt();
    assert!(prompt.contains("<memory>"));
    assert!(prompt.contains("Luna the dog"));
    assert!(prompt.contains("</memory>"));
}

#[test]
fn build_prompt_without_global_skips_wrapper() {
    let mem = test_memory();
    let prompt = mem.build_prompt();
    assert!(!prompt.contains("<memory>"));
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

#[test]
fn migration_imports_legacy_entries_and_index() {
    // Arrange a legacy on-disk layout: config_dir/memory/entries/*.md
    // + config_dir/memory/MEMORY.md.
    let dir = tempdir().unwrap();
    let config_dir = dir.path();
    let legacy = config_dir.join("memory");
    let entries = legacy.join("entries");
    fs::create_dir_all(&entries).unwrap();

    fs::write(
        entries.join("luna.md"),
        "---\nname: luna\ndescription: User's dog Luna is a golden retriever\n---\n\nLuna has vet visits on Thursdays.\n",
    )
    .unwrap();
    fs::write(
        entries.join("rust.md"),
        "---\nname: rust\ndescription: User works on crabtalk\n---\n\nCrabtalk is an AI companion.\n",
    )
    .unwrap();
    fs::write(legacy.join("MEMORY.md"), "# My overview\n\nSome prose.").unwrap();

    let mem = Memory::open(
        MemoryConfig::default(),
        config_dir.join("memory.db"),
        Some(legacy),
    )
    .unwrap();

    let golden = mem.recall("golden retriever", 5);
    assert!(golden.contains("Luna"));
    let rust = mem.recall("crabtalk", 5);
    assert!(rust.contains("Crabtalk"));

    let prompt = mem.build_prompt();
    assert!(prompt.contains("My overview"));
}

#[test]
fn remember_rejects_reserved_global_name() {
    let mem = test_memory();
    let result = mem.remember("global".into(), "hijacked".into(), vec![]);
    assert!(result.contains("reserved"), "got: {result}");
    // Prompt is still empty — the system-prompt wrapper must be absent.
    assert!(!mem.build_prompt().contains("<memory>"));
}

#[test]
fn forget_rejects_reserved_global_name() {
    let mem = test_memory();
    mem.write_prompt("# overview");
    let result = mem.forget("global");
    assert!(result.contains("reserved"), "got: {result}");
    // Prompt content survives.
    assert!(mem.build_prompt().contains("overview"));
}

#[test]
fn migration_creates_db_even_when_all_legacy_entries_malformed() {
    let dir = tempdir().unwrap();
    let config_dir = dir.path();
    let legacy = config_dir.join("memory");
    let entries = legacy.join("entries");
    fs::create_dir_all(&entries).unwrap();

    // All legacy entries lack frontmatter — every `Op::Add` will be
    // skipped, so without the unconditional checkpoint the db file
    // would never be created and the next open would re-enter
    // migration.
    fs::write(entries.join("broken1.md"), "no frontmatter here").unwrap();
    fs::write(entries.join("broken2.md"), "plain text").unwrap();

    let db = config_dir.join("memory.db");
    {
        let _ = Memory::open(MemoryConfig::default(), db.clone(), Some(legacy.clone())).unwrap();
    }
    assert!(db.exists(), "db file must exist after migration attempt");

    // Even a later-added valid legacy entry should NOT be imported —
    // migration has already happened.
    fs::write(
        entries.join("valid.md"),
        "---\nname: valid\ndescription: new\n---\n\nbody\n",
    )
    .unwrap();
    let mem = Memory::open(MemoryConfig::default(), db, Some(legacy)).unwrap();
    assert_eq!(mem.recall("new", 5), "no memories found");
}

#[test]
fn migration_skipped_when_db_already_exists() {
    let dir = tempdir().unwrap();
    let config_dir = dir.path();
    let legacy = config_dir.join("memory");
    let entries = legacy.join("entries");
    fs::create_dir_all(&entries).unwrap();
    fs::write(
        entries.join("existing.md"),
        "---\nname: existing\ndescription: should not be imported\n---\n\nlegacy content\n",
    )
    .unwrap();

    // First open creates an empty db file.
    {
        let _ = Memory::open(
            MemoryConfig::default(),
            config_dir.join("memory.db"),
            Some(legacy.clone()),
        )
        .unwrap();
    }

    // Re-opening with legacy set must NOT re-import — the file already exists.
    // To verify: the migration happens once. Mutate legacy to add a new entry,
    // reopen, and confirm the new entry is absent.
    fs::write(
        entries.join("late.md"),
        "---\nname: late\ndescription: added after first open\n---\n\nshould be skipped\n",
    )
    .unwrap();

    let mem = Memory::open(
        MemoryConfig::default(),
        config_dir.join("memory.db"),
        Some(legacy),
    )
    .unwrap();

    let result = mem.recall("skipped", 5);
    assert_eq!(
        result, "no memories found",
        "migration should not re-run on an existing db"
    );
}
