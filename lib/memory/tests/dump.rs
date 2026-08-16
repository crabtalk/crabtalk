use crabtalk_memory::{EntryKind, Memory, Op};
use std::fs;
use tempfile::tempdir;

fn add(mem: &mut Memory, kind: EntryKind, name: &str, content: &str, aliases: &[&str]) {
    mem.apply(Op::Add {
        name: name.to_owned(),
        content: content.to_owned(),
        aliases: aliases.iter().map(|s| (*s).to_owned()).collect(),
        kind,
    })
    .unwrap();
}

#[test]
fn dump_creates_sections_and_summary() {
    let dir = tempdir().unwrap();
    let brain = dir.path().join("brain");
    let mut mem = Memory::new();
    add(
        &mut mem,
        EntryKind::Note,
        "rust-style",
        "group imports",
        &[],
    );
    add(
        &mut mem,
        EntryKind::Archive,
        "20260401-session42",
        "compacted summary",
        &[],
    );

    mem.dump(&brain).unwrap();

    assert!(brain.join("notes/rust-style.md").is_file());
    assert!(brain.join("archives/20260401-session42.md").is_file());

    let summary = fs::read_to_string(brain.join("SUMMARY.md")).unwrap();
    assert!(summary.contains("# Notes"));
    assert!(summary.contains("# Archives"));
    assert!(summary.contains("[rust-style](notes/rust-style.md)"));
    assert!(summary.contains("[20260401-session42](archives/20260401-session42.md)"));
}

#[test]
fn dump_always_emits_meta_block_with_created() {
    let dir = tempdir().unwrap();
    let brain = dir.path().join("brain");
    let mut mem = Memory::new();
    add(&mut mem, EntryKind::Note, "plain", "just prose", &[]);

    mem.dump(&brain).unwrap();
    let text = fs::read_to_string(brain.join("notes/plain.md")).unwrap();
    assert!(text.starts_with("<div id=\"meta\">"));
    assert!(text.contains("<dl>"));
    assert!(text.contains("<dt>Created</dt>"));
    assert!(text.contains("<time datetime=\""));
    // No aliases row when there are no aliases.
    assert!(!text.contains("<dt>Aliases</dt>"));
    assert!(text.contains("just prose"));
}

#[test]
fn dump_emits_aliases_when_present() {
    let dir = tempdir().unwrap();
    let brain = dir.path().join("brain");
    let mut mem = Memory::new();
    add(
        &mut mem,
        EntryKind::Note,
        "tagged",
        "prod rollout steps",
        &["ship", "release"],
    );

    mem.dump(&brain).unwrap();
    let text = fs::read_to_string(brain.join("notes/tagged.md")).unwrap();
    assert!(text.contains("<dt>Aliases</dt>"));
    assert!(text.contains("<li>ship</li>"));
    assert!(text.contains("<li>release</li>"));
}

#[test]
fn round_trip_preserves_content_aliases_and_timestamp() {
    let dir = tempdir().unwrap();
    let brain = dir.path().join("brain");
    let mut mem = Memory::new();
    add(&mut mem, EntryKind::Note, "a", "line one\nline two", &[]);
    add(
        &mut mem,
        EntryKind::Archive,
        "arc",
        "session text",
        &["session-42", "compact"],
    );
    let created_before: Vec<_> = mem.list().map(|e| (e.name.clone(), e.created_at)).collect();
    mem.dump(&brain).unwrap();

    let mut loaded = Memory::new();
    loaded.load(&brain).unwrap();

    assert_eq!(loaded.list().count(), 2);

    let a = loaded.get("a").unwrap();
    assert_eq!(a.content, "line one\nline two");
    assert_eq!(a.kind, EntryKind::Note);
    assert!(a.aliases.is_empty());

    let arc = loaded.get("arc").unwrap();
    assert_eq!(arc.kind, EntryKind::Archive);
    assert_eq!(arc.aliases, vec!["session-42", "compact"]);

    for (name, ts) in created_before {
        assert_eq!(loaded.get(&name).unwrap().created_at, ts);
    }
}

#[test]
fn load_rebuilds_search_index() {
    let dir = tempdir().unwrap();
    let brain = dir.path().join("brain");
    let mut mem = Memory::new();
    add(
        &mut mem,
        EntryKind::Note,
        "fox",
        "quick brown fox",
        &["animal"],
    );
    mem.dump(&brain).unwrap();

    let mut loaded = Memory::new();
    loaded.load(&brain).unwrap();
    let hits = loaded.search("animal", 5);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entry.name, "fox");
}

#[test]
fn load_replaces_existing_entries() {
    let dir = tempdir().unwrap();
    let brain = dir.path().join("brain");
    let mut src = Memory::new();
    add(&mut src, EntryKind::Note, "keeper", "new content", &[]);
    src.dump(&brain).unwrap();

    let mut mem = Memory::new();
    add(&mut mem, EntryKind::Note, "goner", "old content", &[]);
    mem.load(&brain).unwrap();

    assert!(mem.get("goner").is_none());
    assert!(mem.get("keeper").is_some());
}

#[test]
fn load_ignores_summary_and_non_markdown() {
    let dir = tempdir().unwrap();
    let brain = dir.path().join("brain");
    fs::create_dir_all(brain.join("notes")).unwrap();
    fs::write(brain.join("SUMMARY.md"), "# Summary").unwrap();
    fs::write(brain.join("notes/real.md"), "real note").unwrap();
    fs::write(brain.join("notes/ignored.txt"), "not markdown").unwrap();

    let mut mem = Memory::new();
    mem.load(&brain).unwrap();
    assert_eq!(mem.list().count(), 1);
    assert!(mem.get("real").is_some());
}

#[test]
fn entry_without_meta_block_is_pure_content() {
    let dir = tempdir().unwrap();
    let brain = dir.path().join("brain");
    fs::create_dir_all(brain.join("notes")).unwrap();
    fs::write(brain.join("notes/plain.md"), "hello world\n").unwrap();

    let mut mem = Memory::new();
    mem.load(&brain).unwrap();
    let e = mem.get("plain").unwrap();
    assert_eq!(e.content, "hello world");
    assert!(e.aliases.is_empty());
}

#[test]
fn malformed_meta_falls_back_to_content_only() {
    // Opens a meta div but never closes it — treat the file as pure
    // content, no metadata.
    let dir = tempdir().unwrap();
    let brain = dir.path().join("brain");
    fs::create_dir_all(brain.join("notes")).unwrap();
    fs::write(
        brain.join("notes/bad.md"),
        "<div id=\"meta\">\nmissing close\nhello\n",
    )
    .unwrap();

    let mut mem = Memory::new();
    mem.load(&brain).unwrap();
    let e = mem.get("bad").unwrap();
    assert!(e.content.contains("missing close"));
    assert!(e.aliases.is_empty());
}

#[test]
fn html_entities_round_trip_in_aliases() {
    let dir = tempdir().unwrap();
    let brain = dir.path().join("brain");
    let mut mem = Memory::new();
    add(
        &mut mem,
        EntryKind::Note,
        "weird",
        "content",
        &["a<b", "x&y"],
    );
    mem.dump(&brain).unwrap();
    let text = fs::read_to_string(brain.join("notes/weird.md")).unwrap();
    assert!(text.contains("a&lt;b"));
    assert!(text.contains("x&amp;y"));

    let mut loaded = Memory::new();
    loaded.load(&brain).unwrap();
    assert_eq!(loaded.get("weird").unwrap().aliases, vec!["a<b", "x&y"]);
}

#[test]
fn redump_clears_orphans() {
    let dir = tempdir().unwrap();
    let brain = dir.path().join("brain");

    let mut mem = Memory::new();
    add(&mut mem, EntryKind::Note, "alpha", "first", &[]);
    add(&mut mem, EntryKind::Note, "beta", "second", &[]);
    mem.dump(&brain).unwrap();
    assert!(brain.join("notes/alpha.md").exists());
    assert!(brain.join("notes/beta.md").exists());

    mem.apply(Op::Remove {
        name: "alpha".into(),
    })
    .unwrap();
    mem.dump(&brain).unwrap();

    assert!(!brain.join("notes/alpha.md").exists());
    assert!(brain.join("notes/beta.md").exists());
}

#[test]
fn load_duplicate_leaves_state_untouched() {
    let dir = tempdir().unwrap();
    let brain = dir.path().join("brain");
    fs::create_dir_all(brain.join("notes")).unwrap();
    fs::create_dir_all(brain.join("archives")).unwrap();
    fs::write(brain.join("notes/dup.md"), "a").unwrap();
    fs::write(brain.join("archives/dup.md"), "b").unwrap();

    let mut mem = Memory::new();
    add(&mut mem, EntryKind::Note, "survivor", "stays put", &[]);
    assert!(mem.load(&brain).is_err());
    assert!(mem.get("survivor").is_some());
}

#[test]
fn invalid_name_rejected_by_dump() {
    let dir = tempdir().unwrap();
    let brain = dir.path().join("brain");
    let mut mem = Memory::new();
    add(&mut mem, EntryKind::Note, "bad/name", "x", &[]);
    assert!(mem.dump(&brain).is_err());
}

#[test]
fn persistent_db_dumps_and_reloads_through_tree() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("mem.db");
    let brain = dir.path().join("brain");

    let mut mem = Memory::open(&db).unwrap();
    add(&mut mem, EntryKind::Note, "persistent", "stays", &["alias"]);
    mem.dump(&brain).unwrap();
    mem.load(&brain).unwrap();

    let reopened = Memory::open(&db).unwrap();
    let e = reopened.get("persistent").unwrap();
    assert_eq!(e.content, "stays");
    assert_eq!(e.aliases, vec!["alias"]);
}

#[test]
fn hand_written_meta_loads_created_at() {
    let dir = tempdir().unwrap();
    let brain = dir.path().join("brain");
    fs::create_dir_all(brain.join("notes")).unwrap();
    // 2020-01-01T00:00:00Z = 1577836800
    fs::write(
        brain.join("notes/dated.md"),
        concat!(
            "<div id=\"meta\">\n",
            "<dl>\n",
            "  <dt>Created</dt>\n",
            "  <dd><time datetime=\"2020-01-01T00:00:00Z\">2020-01-01</time></dd>\n",
            "</dl>\n",
            "</div>\n\n",
            "historical content\n",
        ),
    )
    .unwrap();

    let mut mem = Memory::new();
    mem.load(&brain).unwrap();
    let e = mem.get("dated").unwrap();
    assert_eq!(e.created_at, 1577836800);
    assert_eq!(e.content, "historical content");
}
