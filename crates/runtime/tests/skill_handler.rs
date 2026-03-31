//! Tests for SkillHandler — skill loading from directories.

use crabtalk_runtime::SkillHandler;
use std::fs;
use tempfile::TempDir;

fn write_skill(dir: &std::path::Path, name: &str) {
    let skill_dir = dir.join(name);
    fs::create_dir_all(&skill_dir).unwrap();
    let content =
        format!("---\nname: {name}\ndescription: test skill\n---\nSkill body for {name}.");
    fs::write(skill_dir.join("SKILL.md"), content).unwrap();
}

#[test]
fn load_from_single_dir() {
    let dir = TempDir::new().unwrap();
    write_skill(dir.path(), "greet");
    write_skill(dir.path(), "search");

    let handler = SkillHandler::load(vec![dir.path().to_path_buf()], &[]).unwrap();
    let reg = handler.registry.blocking_lock();
    assert_eq!(reg.skills.len(), 2);
    assert!(reg.contains("greet"));
    assert!(reg.contains("search"));
}

#[test]
fn load_from_multiple_dirs() {
    let dir1 = TempDir::new().unwrap();
    let dir2 = TempDir::new().unwrap();
    write_skill(dir1.path(), "skill-a");
    write_skill(dir2.path(), "skill-b");

    let handler = SkillHandler::load(
        vec![dir1.path().to_path_buf(), dir2.path().to_path_buf()],
        &[],
    )
    .unwrap();
    let reg = handler.registry.blocking_lock();
    assert_eq!(reg.skills.len(), 2);
}

#[test]
fn load_skips_missing_dir() {
    let dir = TempDir::new().unwrap();
    write_skill(dir.path(), "exists");

    let handler = SkillHandler::load(
        vec![
            std::path::PathBuf::from("/nonexistent/path"),
            dir.path().to_path_buf(),
        ],
        &[],
    )
    .unwrap();
    let reg = handler.registry.blocking_lock();
    assert_eq!(reg.skills.len(), 1);
    assert!(reg.contains("exists"));
}

#[test]
fn load_empty_dir() {
    let dir = TempDir::new().unwrap();
    let handler = SkillHandler::load(vec![dir.path().to_path_buf()], &[]).unwrap();
    let reg = handler.registry.blocking_lock();
    assert!(reg.skills.is_empty());
}

#[test]
fn load_conflict_first_dir_wins() {
    let dir1 = TempDir::new().unwrap();
    let dir2 = TempDir::new().unwrap();

    // Write different bodies so we can tell which one loaded
    let skill_dir = dir1.path().join("shared");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: shared\ndescription: from dir1\n---\nfirst body",
    )
    .unwrap();

    let skill_dir = dir2.path().join("shared");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: shared\ndescription: from dir2\n---\nsecond body",
    )
    .unwrap();

    let handler = SkillHandler::load(
        vec![dir1.path().to_path_buf(), dir2.path().to_path_buf()],
        &[],
    )
    .unwrap();
    let reg = handler.registry.blocking_lock();
    assert_eq!(reg.skills.len(), 1);
    // First dir wins — verify it's from dir1
    assert_eq!(reg.skills[0].description, "from dir1");
}

#[test]
fn load_skips_hidden_dirs() {
    let dir = TempDir::new().unwrap();
    write_skill(dir.path(), "visible");
    // Create a hidden directory
    let hidden = dir.path().join(".hidden");
    fs::create_dir_all(&hidden).unwrap();
    fs::write(
        hidden.join("SKILL.md"),
        "---\nname: hidden\ndescription: x\n---\nbody",
    )
    .unwrap();

    let handler = SkillHandler::load(vec![dir.path().to_path_buf()], &[]).unwrap();
    let reg = handler.registry.blocking_lock();
    assert_eq!(reg.skills.len(), 1);
    assert!(reg.contains("visible"));
    assert!(!reg.contains("hidden"));
}

#[test]
fn load_no_dirs() {
    let handler = SkillHandler::load(vec![], &[]).unwrap();
    let reg = handler.registry.blocking_lock();
    assert!(reg.skills.is_empty());
}
