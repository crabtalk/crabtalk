use crabtalk::{Config, storage::DEFAULT_CONFIG};

#[test]
fn parse_default_config_template() {
    Config::from_toml(DEFAULT_CONFIG).expect("default config template should parse");
}

#[test]
fn empty_config() {
    let config = Config::from_toml("").unwrap();
    assert!(config.llm.base_url.is_empty());
    assert!(config.llm.api_key.is_empty());
    assert!(config.env.is_empty());
}

#[test]
fn invalid_toml_syntax() {
    let result = Config::from_toml("this is not [valid toml");
    assert!(result.is_err());
}

#[test]
fn task_defaults() {
    let config = Config::from_toml("").unwrap();
    assert_eq!(config.tasks.max_concurrent, 4);
    assert_eq!(config.tasks.viewable_window, 16);
    assert_eq!(config.tasks.task_timeout, 300);
}

#[test]
fn task_overrides() {
    let toml = r#"
[tasks]
max_concurrent = 8
task_timeout = 600
"#;
    let config = Config::from_toml(toml).unwrap();
    assert_eq!(config.tasks.max_concurrent, 8);
    assert_eq!(config.tasks.task_timeout, 600);
}

#[test]
fn env_vars_parsed() {
    let toml = r#"
[env]
FOO = "bar"
BAZ = "qux"
"#;
    let config = Config::from_toml(toml).unwrap();
    assert_eq!(config.env.get("FOO").unwrap(), "bar");
    assert_eq!(config.env.get("BAZ").unwrap(), "qux");
}

#[test]
fn llm_section_parsed() {
    let toml = r#"
[llm]
base_url = "http://localhost:4000/v1"
api_key = "sk-test"
"#;
    let config = Config::from_toml(toml).unwrap();
    assert_eq!(config.llm.base_url, "http://localhost:4000/v1");
    assert_eq!(config.llm.api_key, "sk-test");
}

/// `[mcps.*]` and `[agents.*]` are mutable runtime records and live in
/// `local/settings.toml`, not config.toml. The parser ignores any
/// stray sections (TOML allows unknown keys via `#[serde(default)]`),
/// so a hand-edit doesn't crash the daemon — but the values are
/// silently dropped, which is the intended behavior.
#[test]
fn unknown_sections_ignored() {
    let toml = r#"
[mcps.myserver]
command = "my-mcp-server"

[agents.helper]
description = "A helper agent"
"#;
    Config::from_toml(toml).unwrap();
}
