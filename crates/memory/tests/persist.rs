use crabtalk_memory::{EntryKind, Memory, Op};
use std::fs;
use tempfile::tempdir;

fn add(mem: &mut Memory, name: &str, content: &str, aliases: &[&str], kind: EntryKind) {
    mem.apply(Op::Add {
        name: name.to_owned(),
        content: content.to_owned(),
        aliases: aliases.iter().map(|s| (*s).to_owned()).collect(),
        kind,
    })
    .unwrap();
}

#[test]
fn open_missing_file_starts_empty() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("mem.db");
    let mem = Memory::open(&path).unwrap();
    assert_eq!(mem.list().count(), 0);
    // File is not created until first write.
    assert!(!path.exists());
}

#[test]
fn round_trip_preserves_entries() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("mem.db");

    {
        let mut mem = Memory::open(&path).unwrap();
        add(
            &mut mem,
            "rust",
            "ownership and borrowing",
            &["borrow"],
            EntryKind::Note,
        );
        add(
            &mut mem,
            "archive-1",
            "session summary text",
            &[],
            EntryKind::Archive,
        );
    }

    let mem = Memory::open(&path).unwrap();
    assert_eq!(mem.list().count(), 2);

    let rust = mem.get("rust").unwrap();
    assert_eq!(rust.content, "ownership and borrowing");
    assert_eq!(rust.aliases, vec!["borrow"]);
    assert_eq!(rust.kind, EntryKind::Note);
    assert!(rust.created_at > 0);

    let arch = mem.get("archive-1").unwrap();
    assert_eq!(arch.kind, EntryKind::Archive);
}

#[test]
fn reopened_index_serves_search() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("mem.db");

    {
        let mut mem = Memory::open(&path).unwrap();
        add(&mut mem, "a", "quick brown fox", &[], EntryKind::Note);
        add(
            &mut mem,
            "b",
            "lazy dog sleeping",
            &["nap"],
            EntryKind::Note,
        );
    }

    let mem = Memory::open(&path).unwrap();
    let hits = mem.search("nap", 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entry.name, "b");

    let hits = mem.search("fox", 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entry.name, "a");
}

#[test]
fn next_id_survives_round_trip() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("mem.db");

    let first_id = {
        let mut mem = Memory::open(&path).unwrap();
        add(&mut mem, "a", "x", &[], EntryKind::Note);
        add(&mut mem, "b", "y", &[], EntryKind::Note);
        mem.apply(Op::Remove { name: "a".into() }).unwrap();
        mem.get("b").unwrap().id
    };

    let mut mem = Memory::open(&path).unwrap();
    add(&mut mem, "c", "z", &[], EntryKind::Note);
    let c_id = mem.get("c").unwrap().id;
    // ids must be monotonic across sessions, so 'c' gets a higher id
    // than anything that ever lived in the db.
    assert!(c_id > first_id);
}

#[test]
fn update_and_remove_persist() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("mem.db");

    {
        let mut mem = Memory::open(&path).unwrap();
        add(&mut mem, "x", "v1", &[], EntryKind::Note);
        mem.apply(Op::Update {
            name: "x".into(),
            content: "v2".into(),
            aliases: vec!["xx".into()],
        })
        .unwrap();
        add(&mut mem, "gone", "bye", &[], EntryKind::Note);
        mem.apply(Op::Remove {
            name: "gone".into(),
        })
        .unwrap();
    }

    let mem = Memory::open(&path).unwrap();
    let x = mem.get("x").unwrap();
    assert_eq!(x.content, "v2");
    assert_eq!(x.aliases, vec!["xx"]);
    assert!(mem.get("gone").is_none());
}

#[test]
fn bad_magic_is_rejected() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("mem.db");
    fs::write(&path, b"NOTMEMFILE______garbage").unwrap();
    let err = match Memory::open(&path) {
        Ok(_) => panic!("expected bad magic rejection"),
        Err(e) => e,
    };
    assert!(format!("{err}").contains("bad memory file format"));
}

#[test]
fn truncated_file_is_rejected() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("mem.db");
    fs::write(&path, b"CRMEM\0").unwrap();
    assert!(Memory::open(&path).is_err());
}

#[test]
fn unknown_version_is_rejected() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("mem.db");
    let mut bad = Vec::new();
    bad.extend_from_slice(b"CRMEM\0");
    bad.extend_from_slice(&99u32.to_le_bytes()); // version
    bad.extend_from_slice(&[0u8; 6]); // flags + reserved
    fs::write(&path, &bad).unwrap();
    assert!(Memory::open(&path).is_err());
}

#[test]
fn multi_alias_round_trip() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("mem.db");
    {
        let mut mem = Memory::open(&path).unwrap();
        add(
            &mut mem,
            "deploy",
            "prod rollout",
            &["ship", "release", "cut"],
            EntryKind::Note,
        );
    }
    let mem = Memory::open(&path).unwrap();
    let e = mem.get("deploy").unwrap();
    assert_eq!(e.aliases, vec!["ship", "release", "cut"]);
}

#[test]
fn unicode_round_trip() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("mem.db");
    {
        let mut mem = Memory::open(&path).unwrap();
        add(
            &mut mem,
            "螃蟹",
            "crabs all the way 🦀",
            &["カニ"],
            EntryKind::Note,
        );
    }
    let mem = Memory::open(&path).unwrap();
    let e = mem.get("螃蟹").unwrap();
    assert_eq!(e.content, "crabs all the way 🦀");
    assert_eq!(e.aliases, vec!["カニ"]);
}

#[test]
fn empty_content_round_trips() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("mem.db");
    {
        let mut mem = Memory::open(&path).unwrap();
        add(&mut mem, "blank", "", &[], EntryKind::Note);
    }
    let mem = Memory::open(&path).unwrap();
    assert_eq!(mem.get("blank").unwrap().content, "");
}

/// Byte-for-byte fixture. Regression guard against silent format drift.
/// A single entry: id=1, created_at=0x1122334455667788, kind=Archive,
/// name="hi", content="yo", one alias "hey". next_id=2.
#[test]
fn known_good_fixture() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("mem.db");

    let mut bytes = Vec::new();
    // header
    bytes.extend_from_slice(b"CRMEM\0");
    bytes.extend_from_slice(&1u32.to_le_bytes()); // version
    bytes.extend_from_slice(&[0u8; 6]); // flags + reserved
    // body
    bytes.extend_from_slice(&2u64.to_le_bytes()); // next_id
    bytes.extend_from_slice(&1u32.to_le_bytes()); // entry_count
    // entry
    bytes.extend_from_slice(&1u64.to_le_bytes()); // id
    bytes.extend_from_slice(&0x1122334455667788u64.to_le_bytes()); // created_at
    bytes.extend_from_slice(&1u32.to_le_bytes()); // kind = Archive
    bytes.extend_from_slice(&2u32.to_le_bytes()); // name len
    bytes.extend_from_slice(b"hi");
    bytes.extend_from_slice(&2u32.to_le_bytes()); // content len
    bytes.extend_from_slice(b"yo");
    bytes.extend_from_slice(&1u32.to_le_bytes()); // alias count
    bytes.extend_from_slice(&3u32.to_le_bytes()); // alias len
    bytes.extend_from_slice(b"hey");

    fs::write(&path, &bytes).unwrap();
    let mem = Memory::open(&path).unwrap();
    let e = mem.get("hi").unwrap();
    assert_eq!(e.id, 1);
    assert_eq!(e.created_at, 0x1122334455667788);
    assert_eq!(e.kind, EntryKind::Archive);
    assert_eq!(e.content, "yo");
    assert_eq!(e.aliases, vec!["hey"]);
}
