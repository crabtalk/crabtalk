use crabtalk::{NodeConfig, storage::DEFAULT_CONFIG};

#[test]
fn parse_default_config_template() {
    NodeConfig::from_toml(DEFAULT_CONFIG).expect("default config template should parse");
}

#[test]
fn empty_config() {
    let config = NodeConfig::from_toml("").unwrap();
    assert!(config.provider.is_empty());
    assert!(config.mcps.is_empty());
    assert!(config.env.is_empty());
}

#[test]
fn invalid_toml_syntax() {
    let result = NodeConfig::from_toml("this is not [valid toml");
    assert!(result.is_err());
}

#[test]
fn system_defaults() {
    let config = NodeConfig::from_toml("").unwrap();
    assert_eq!(config.system.tasks.max_concurrent, 4);
    assert_eq!(config.system.tasks.viewable_window, 16);
    assert_eq!(config.system.tasks.task_timeout, 300);
    assert_eq!(config.system.memory.recall_limit, 5);
}

#[test]
fn system_overrides() {
    let toml = r#"
[system.tasks]
max_concurrent = 8
task_timeout = 600

[system.memory]
recall_limit = 10
"#;
    let config = NodeConfig::from_toml(toml).unwrap();
    assert_eq!(config.system.tasks.max_concurrent, 8);
    assert_eq!(config.system.tasks.task_timeout, 600);
    assert_eq!(config.system.memory.recall_limit, 10);
}

#[test]
fn env_vars_parsed() {
    let toml = r#"
[env]
FOO = "bar"
BAZ = "qux"
"#;
    let config = NodeConfig::from_toml(toml).unwrap();
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
    let config = NodeConfig::from_toml(toml).unwrap();
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
    let config = NodeConfig::from_toml(toml).unwrap();
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
    NodeConfig::from_toml(toml).unwrap();
}
