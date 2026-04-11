//! Tests for skill storage and lookup via InMemoryStorage.

use wcore::{
    repos::{Skill, Storage},
    test_utils::InMemoryStorage,
};

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
    let s = InMemoryStorage::with_skills(vec![skill("greet", "greet"), skill("search", "search")]);
    let skills = s.list_skills().unwrap();
    assert_eq!(skills.len(), 2);
}

#[test]
fn load_existing_skill() {
    let s = InMemoryStorage::with_skills(vec![skill("greet", "greet")]);
    let loaded = s.load_skill("greet").unwrap();
    assert!(loaded.is_some());
    assert_eq!(loaded.unwrap().name, "greet");
}

#[test]
fn load_missing_skill() {
    let s = InMemoryStorage::with_skills(vec![skill("greet", "greet")]);
    let loaded = s.load_skill("missing").unwrap();
    assert!(loaded.is_none());
}

#[test]
fn empty_storage() {
    let s = InMemoryStorage::new();
    let skills = s.list_skills().unwrap();
    assert!(skills.is_empty());
}
