//! Tests for SkillRegistry — skill storage and lookup.

use crabtalk_runtime::skill::{Skill, SkillRegistry};
use std::collections::BTreeMap;

fn skill(name: &str) -> Skill {
    Skill {
        name: name.into(),
        description: format!("{name} skill"),
        license: None,
        compatibility: None,
        metadata: BTreeMap::new(),
        allowed_tools: Vec::new(),
        body: format!("body of {name}"),
    }
}

#[test]
fn new_is_empty() {
    let reg = SkillRegistry::new();
    assert!(reg.skills.is_empty());
    assert_eq!(reg.skills.len(), 0);
}

#[test]
fn add_and_contains() {
    let mut reg = SkillRegistry::new();
    reg.add(skill("greet"));
    assert!(reg.contains("greet"));
    assert!(!reg.contains("other"));
    assert_eq!(reg.skills.len(), 1);
}

#[test]
fn add_multiple() {
    let mut reg = SkillRegistry::new();
    reg.add(skill("a"));
    reg.add(skill("b"));
    reg.add(skill("c"));
    assert_eq!(reg.skills.len(), 3);
}

#[test]
fn skills_returns_all() {
    let mut reg = SkillRegistry::new();
    reg.add(skill("x"));
    reg.add(skill("y"));
    let skills = reg.skills.clone();
    assert_eq!(skills.len(), 2);
    let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"x"));
    assert!(names.contains(&"y"));
}

#[test]
fn upsert_adds_new() {
    let mut reg = SkillRegistry::new();
    reg.upsert(skill("new"));
    assert!(reg.contains("new"));
    assert_eq!(reg.skills.len(), 1);
}

#[test]
fn upsert_replaces_existing() {
    let mut reg = SkillRegistry::new();
    let mut s = skill("greet");
    s.body = "old body".into();
    reg.add(s);

    let mut updated = skill("greet");
    updated.body = "new body".into();
    reg.upsert(updated);

    assert_eq!(reg.skills.len(), 1);
    let skills = reg.skills.clone();
    assert_eq!(skills[0].body, "new body");
}

#[test]
fn contains_is_false_for_absent() {
    let reg = SkillRegistry::new();
    assert!(!reg.contains("nonexistent"));
}

#[test]
fn add_allows_duplicates() {
    // add() appends without dedup — callers must check contains() first.
    // This test documents the current behavior.
    let mut reg = SkillRegistry::new();
    reg.add(skill("dup"));
    reg.add(skill("dup"));
    assert_eq!(reg.skills.len(), 2);
    // upsert() is the correct way to avoid duplicates
    let mut reg2 = SkillRegistry::new();
    reg2.upsert(skill("dup"));
    reg2.upsert(skill("dup"));
    assert_eq!(reg2.skills.len(), 1);
}
