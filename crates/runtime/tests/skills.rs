//! Tests for SkillRegistry.

use agent::{Skill, SkillTier};
use compact_str::CompactString;
use std::collections::BTreeMap;
use walrus_runtime::parse_skill_md;
use walrus_runtime::skills::SkillRegistry;

#[test]
fn parse_skill_frontmatter() {
    let content = "\
---
name: code-helper
description: Helps write code.
license: MIT
metadata:
  tags: coding,rust
  triggers: help me code,write code
allowed-tools: bash read_file
---

You are a coding assistant.
";
    let skill = parse_skill_md(content).unwrap();
    assert_eq!(skill.name.as_str(), "code-helper");
    assert_eq!(skill.description, "Helps write code.");
    assert_eq!(skill.license.as_deref(), Some("MIT"));
    assert_eq!(skill.metadata.get("tags").unwrap(), "coding,rust");
    assert_eq!(skill.allowed_tools.len(), 2);
    assert_eq!(skill.allowed_tools[0].as_str(), "bash");
    assert_eq!(skill.allowed_tools[1].as_str(), "read_file");
    assert!(skill.body.contains("coding assistant"));
}

fn make_skill(name: &str, tags: &str, triggers: &str, priority: u8) -> Skill {
    let mut metadata = BTreeMap::new();
    if !tags.is_empty() {
        metadata.insert(CompactString::from("tags"), tags.into());
    }
    if !triggers.is_empty() {
        metadata.insert(CompactString::from("triggers"), triggers.into());
    }
    metadata.insert(CompactString::from("priority"), priority.to_string());
    Skill {
        name: name.into(),
        description: String::new(),
        license: None,
        compatibility: None,
        metadata,
        allowed_tools: vec![],
        body: format!("Body of {name}"),
    }
}

#[test]
fn load_dir_discovers_skills() {
    let dir = tempfile::tempdir().unwrap();

    // Create two skill directories.
    let skill_a = dir.path().join("skill-a");
    std::fs::create_dir(&skill_a).unwrap();
    std::fs::write(
        skill_a.join("SKILL.md"),
        "---\nname: skill-a\ndescription: A\n---\nBody A\n",
    )
    .unwrap();

    let skill_b = dir.path().join("skill-b");
    std::fs::create_dir(&skill_b).unwrap();
    std::fs::write(
        skill_b.join("SKILL.md"),
        "---\nname: skill-b\ndescription: B\n---\nBody B\n",
    )
    .unwrap();

    // A non-skill file in the directory should be ignored.
    std::fs::write(dir.path().join("README.md"), "not a skill").unwrap();

    let registry = SkillRegistry::load_dir(dir.path(), SkillTier::Workspace).unwrap();
    assert_eq!(registry.len(), 2);
}

#[test]
fn find_by_tags_returns_matches() {
    let mut registry = SkillRegistry::new();
    registry.add(make_skill("coding", "code,rust", "", 0), SkillTier::Bundled);
    registry.add(
        make_skill("writing", "writing,blog", "", 0),
        SkillTier::Bundled,
    );

    let results = registry.find_by_tags(&["code".into()]);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name.as_str(), "coding");

    let results = registry.find_by_tags(&["writing".into()]);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name.as_str(), "writing");

    // No match.
    let results = registry.find_by_tags(&["unknown".into()]);
    assert!(results.is_empty());
}

#[test]
fn find_by_trigger_case_insensitive() {
    let mut registry = SkillRegistry::new();
    registry.add(
        make_skill("helper", "", "help me,assist", 0),
        SkillTier::Bundled,
    );

    // Trigger keywords stored lowercase, query lowercased.
    let results = registry.find_by_trigger("Can you HELP ME with this?");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name.as_str(), "helper");

    let results = registry.find_by_trigger("I need you to ASSIST me");
    assert_eq!(results.len(), 1);

    let results = registry.find_by_trigger("nothing relevant");
    assert!(results.is_empty());
}

#[test]
fn ranking_by_tier_then_priority() {
    let mut registry = SkillRegistry::new();
    registry.add(
        make_skill("bundled-low", "shared", "", 10),
        SkillTier::Bundled,
    );
    registry.add(
        make_skill("managed-mid", "shared", "", 50),
        SkillTier::Managed,
    );
    registry.add(
        make_skill("workspace-high", "shared", "", 5),
        SkillTier::Workspace,
    );
    registry.add(
        make_skill("workspace-low", "shared", "", 1),
        SkillTier::Workspace,
    );

    let results = registry.find_by_tags(&["shared".into()]);
    assert_eq!(results.len(), 4);
    // Workspace first (higher tier), then within workspace by priority desc.
    assert_eq!(results[0].name.as_str(), "workspace-high");
    assert_eq!(results[1].name.as_str(), "workspace-low");
    // Then managed.
    assert_eq!(results[2].name.as_str(), "managed-mid");
    // Then bundled.
    assert_eq!(results[3].name.as_str(), "bundled-low");
}
