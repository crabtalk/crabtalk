use crabtalk_memory::{EntryKind, Memory, Op};

fn add(mem: &mut Memory, name: &str, content: &str, aliases: &[&str]) {
    mem.apply(Op::Add {
        name: name.to_owned(),
        content: content.to_owned(),
        aliases: aliases.iter().map(|s| (*s).to_owned()).collect(),
        kind: EntryKind::Note,
    })
    .unwrap();
}

#[test]
fn add_get_list() {
    let mut mem = Memory::new();
    add(&mut mem, "rust-style", "group imports", &[]);
    add(&mut mem, "commits", "conventional format", &[]);

    assert_eq!(mem.list().count(), 2);
    assert_eq!(mem.get("rust-style").unwrap().content, "group imports");
    assert!(mem.get("missing").is_none());
}

#[test]
fn duplicate_add_errors() {
    let mut mem = Memory::new();
    add(&mut mem, "a", "first", &[]);
    let err = mem.apply(Op::Add {
        name: "a".into(),
        content: "second".into(),
        aliases: vec![],
        kind: EntryKind::Note,
    });
    assert!(err.is_err());
}

#[test]
fn update_replaces_content_and_reindexes() {
    let mut mem = Memory::new();
    add(&mut mem, "note", "apple banana", &[]);
    mem.apply(Op::Update {
        name: "note".into(),
        content: "cherry durian".into(),
        aliases: vec![],
    })
    .unwrap();

    assert_eq!(mem.get("note").unwrap().content, "cherry durian");
    assert!(mem.search("apple", 10).is_empty());
    assert_eq!(mem.search("cherry", 10).len(), 1);
}

#[test]
fn remove_drops_entry_and_index() {
    let mut mem = Memory::new();
    add(&mut mem, "gone", "transient data", &[]);
    mem.apply(Op::Remove {
        name: "gone".into(),
    })
    .unwrap();

    assert!(mem.get("gone").is_none());
    assert!(mem.search("transient", 10).is_empty());
}

#[test]
fn remove_missing_errors() {
    let mut mem = Memory::new();
    let err = mem.apply(Op::Remove {
        name: "nope".into(),
    });
    assert!(err.is_err());
}

#[test]
fn search_ranks_by_bm25() {
    let mut mem = Memory::new();
    add(&mut mem, "a", "rust rust rust memory", &[]);
    add(&mut mem, "b", "rust memory lane", &[]);
    add(&mut mem, "c", "unrelated text entirely", &[]);

    let hits = mem.search("rust memory", 10);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].entry.name, "a");
    assert_eq!(hits[1].entry.name, "b");
}

#[test]
fn aliases_feed_the_index() {
    let mut mem = Memory::new();
    add(
        &mut mem,
        "deploy",
        "prod release steps",
        &["ship", "rollout"],
    );

    let hits = mem.search("ship", 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entry.name, "deploy");
}

#[test]
fn alias_op_updates_without_touching_content() {
    let mut mem = Memory::new();
    add(&mut mem, "deploy", "prod release steps", &["ship"]);
    mem.apply(Op::Alias {
        name: "deploy".into(),
        aliases: vec!["rollout".into()],
    })
    .unwrap();

    assert_eq!(mem.get("deploy").unwrap().aliases, vec!["rollout"]);
    assert!(mem.search("ship", 10).is_empty());
    assert_eq!(mem.search("rollout", 10).len(), 1);
    assert_eq!(mem.search("prod", 10).len(), 1);
}

#[test]
fn archive_kind_is_preserved() {
    let mut mem = Memory::new();
    mem.apply(Op::Add {
        name: "archive-1".into(),
        content: "session summary".into(),
        aliases: vec![],
        kind: EntryKind::Archive,
    })
    .unwrap();

    assert_eq!(mem.get("archive-1").unwrap().kind, EntryKind::Archive);
}
