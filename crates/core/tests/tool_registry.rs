//! Tests for ToolRegistry — schema-only tool store.

use schemars::Schema;
use crabtalk_core::{ToolRegistry, model::Tool};

fn tool(name: &str) -> Tool {
    Tool {
        name: name.into(),
        description: format!("{name} tool"),
        parameters: Schema::default(),
        strict: true,
    }
}

#[test]
fn insert_and_contains() {
    let mut reg = ToolRegistry::new();
    assert!(!reg.contains("bash"));
    reg.insert(tool("bash"));
    assert!(reg.contains("bash"));
    assert_eq!(reg.len(), 1);
}

#[test]
fn insert_all() {
    let mut reg = ToolRegistry::new();
    reg.insert_all(vec![tool("bash"), tool("recall"), tool("delegate")]);
    assert_eq!(reg.len(), 3);
    assert!(reg.contains("bash"));
    assert!(reg.contains("recall"));
    assert!(reg.contains("delegate"));
}

#[test]
fn remove() {
    let mut reg = ToolRegistry::new();
    reg.insert(tool("bash"));
    assert!(reg.remove("bash"));
    assert!(!reg.contains("bash"));
    assert!(reg.is_empty());
}

#[test]
fn remove_nonexistent() {
    let mut reg = ToolRegistry::new();
    assert!(!reg.remove("nonexistent"));
}

#[test]
fn tools_returns_all() {
    let mut reg = ToolRegistry::new();
    reg.insert_all(vec![tool("a"), tool("b"), tool("c")]);
    let tools = reg.tools();
    assert_eq!(tools.len(), 3);
}

#[test]
fn filtered_snapshot_empty_names_returns_all() {
    let mut reg = ToolRegistry::new();
    reg.insert_all(vec![tool("a"), tool("b"), tool("c")]);
    let snapshot = reg.filtered_snapshot(&[]);
    assert_eq!(snapshot.len(), 3);
}

#[test]
fn filtered_snapshot_filters_by_name() {
    let mut reg = ToolRegistry::new();
    reg.insert_all(vec![tool("a"), tool("b"), tool("c")]);
    let names = vec!["a".to_owned(), "c".to_owned()];
    let snapshot = reg.filtered_snapshot(&names);
    assert_eq!(snapshot.len(), 2);
    let names: Vec<&str> = snapshot.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"a"));
    assert!(names.contains(&"c"));
    assert!(!names.contains(&"b"));
}

#[test]
fn filtered_snapshot_ignores_unknown_names() {
    let mut reg = ToolRegistry::new();
    reg.insert(tool("a"));
    let names = vec!["a".to_owned(), "z".to_owned()];
    let snapshot = reg.filtered_snapshot(&names);
    assert_eq!(snapshot.len(), 1);
}

#[test]
fn insert_overwrites_same_name() {
    let mut reg = ToolRegistry::new();
    reg.insert(tool("bash"));
    let mut updated = tool("bash");
    updated.description = "updated".into();
    reg.insert(updated);
    assert_eq!(reg.len(), 1);
    let tools = reg.tools();
    assert_eq!(tools[0].description, "updated");
}
