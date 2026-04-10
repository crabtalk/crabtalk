//! Tests for InMemorySkillRepo — skill storage and lookup.

use wcore::repos::{Skill, SkillRepo, mem::InMemorySkillRepo};

fn skill(name: &str, description: &str) -> Skill {
    Skill {
        name: name.to_owned(),
        description: description.to_owned(),
        license: None,
        compatibility: None,
        metadata: Default::default(),
        allowed_tools: Vec::new(),
        body: format!("Skill body for {name}."),
    }
}

#[test]
fn list_returns_all_skills() {
    let repo =
        InMemorySkillRepo::with_skills(vec![skill("greet", "greet"), skill("search", "search")]);
    let skills = repo.list().unwrap();
    assert_eq!(skills.len(), 2);
}

#[test]
fn load_existing_skill() {
    let repo = InMemorySkillRepo::with_skills(vec![skill("greet", "greet")]);
    let s = repo.load("greet").unwrap();
    assert!(s.is_some());
    assert_eq!(s.unwrap().name, "greet");
}

#[test]
fn load_missing_skill() {
    let repo = InMemorySkillRepo::with_skills(vec![skill("greet", "greet")]);
    let s = repo.load("missing").unwrap();
    assert!(s.is_none());
}

#[test]
fn empty_repo() {
    let repo = InMemorySkillRepo::new();
    let skills = repo.list().unwrap();
    assert!(skills.is_empty());
}
