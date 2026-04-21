use crabtalk::{DaemonConfig, storage::DEFAULT_CONFIG};

#[test]
fn parse_default_config_template() {
    DaemonConfig::from_toml(DEFAULT_CONFIG).expect("default config template should parse");
}

#[test]
fn empty_config() {
    let config = DaemonConfig::from_toml("").unwrap();
    assert!(config.provider.is_empty());
    assert!(config.mcps.is_empty());
    assert!(config.env.is_empty());
}

#[test]
fn invalid_toml_syntax() {
    let result = DaemonConfig::from_toml("this is not [valid toml");
    assert!(result.is_err());
}

#[test]
fn system_defaults() {
    let config = DaemonConfig::from_toml("").unwrap();
    assert_eq!(config.system.tasks.max_concurrent, 4);
    assert_eq!(config.system.tasks.viewable_window, 16);
    assert_eq!(config.system.tasks.task_timeout, 300);
    assert_eq!(config.hooks.memory.recall_limit, 5);
}

#[test]
fn system_overrides() {
    let toml = r#"
[system.tasks]
max_concurrent = 8
task_timeout = 600

[hooks.memory]
recall_limit = 10
"#;
    let config = DaemonConfig::from_toml(toml).unwrap();
    assert_eq!(config.system.tasks.max_concurrent, 8);
    assert_eq!(config.system.tasks.task_timeout, 600);
    assert_eq!(config.hooks.memory.recall_limit, 10);
}

#[test]
fn legacy_system_bash_memory_migrated_to_hooks() {
    let toml = r#"
[system.bash]
disabled = true
deny = [".ssh"]

[system.memory]
recall_limit = 20
"#;
    let config = DaemonConfig::from_toml(toml).unwrap();
    assert!(config.hooks.bash.disabled);
    assert_eq!(config.hooks.bash.deny, vec![".ssh".to_string()]);
    assert_eq!(config.hooks.memory.recall_limit, 20);
    assert!(config.system.legacy_bash.is_none());
    assert!(config.system.legacy_memory.is_none());
}

#[test]
fn both_legacy_and_hooks_set_prefers_hooks() {
    let toml = r#"
[system.bash]
disabled = true

[hooks.bash]
disabled = false
deny = ["rm -rf"]

[system.memory]
recall_limit = 99

[hooks.memory]
recall_limit = 7
"#;
    let config = DaemonConfig::from_toml(toml).unwrap();
    assert!(!config.hooks.bash.disabled);
    assert_eq!(config.hooks.bash.deny, vec!["rm -rf".to_string()]);
    assert_eq!(config.hooks.memory.recall_limit, 7);
}

#[test]
fn env_vars_parsed() {
    let toml = r#"
[env]
FOO = "bar"
BAZ = "qux"
"#;
    let config = DaemonConfig::from_toml(toml).unwrap();
    assert_eq!(config.env.get("FOO").unwrap(), "bar");
    assert_eq!(config.env.get("BAZ").unwrap(), "qux");
}

#[test]
fn provider_section_parsed() {
    let toml = r#"
[provider.openai]
api_key = "test-key"
models = ["gpt-4o"]
"#;
    let config = DaemonConfig::from_toml(toml).unwrap();
    let p = &config.provider["openai"];
    assert_eq!(p.api_key.as_deref(), Some("test-key"));
    assert_eq!(p.models, vec!["gpt-4o"]);
}

#[test]
fn deprecated_mcps_still_parsed() {
    let toml = r#"
[mcps.myserver]
command = "my-mcp-server"
"#;
    let config = DaemonConfig::from_toml(toml).unwrap();
    let server = &config.mcps["myserver"];
    assert_eq!(server.command, "my-mcp-server");
}

#[test]
fn unknown_agents_section_ignored() {
    let toml = r#"
[agents.helper]
description = "A helper agent"
"#;
    // [agents] is no longer a known field — TOML parser ignores it.
    DaemonConfig::from_toml(toml).unwrap();
}
