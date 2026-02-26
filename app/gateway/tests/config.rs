//! Gateway configuration tests.

use walrus_gateway::{GatewayConfig, config::MemoryBackendKind};

#[test]
fn parse_minimal_config() {
    let toml = r#"
[server]
host = "0.0.0.0"
port = 8080

[llm]
model = "deepseek-chat"
api_key = "test-key"
"#;
    let config = GatewayConfig::from_toml(toml).unwrap();
    assert_eq!(config.server.host, "0.0.0.0");
    assert_eq!(config.server.port, 8080);
    assert_eq!(config.llm.model.as_str(), "deepseek-chat");
    assert_eq!(config.llm.api_key, "test-key");
    assert!(config.agents.is_empty());
    assert!(config.channels.is_empty());
    assert!(config.cron.is_empty());
}

#[test]
fn parse_full_config() {
    let toml = r#"
[server]
host = "0.0.0.0"
port = 3000

[llm]
model = "deepseek-chat"
api_key = "sk-test"

[memory]
backend = "sqlite"
path = "data/memory.db"

[[agents]]
name = "assistant"
description = "A helpful assistant"
system_prompt = "You are helpful."
tools = ["remember"]
skill_tags = ["default"]

[auth]
api_keys = ["key-1", "key-2"]

[[channels]]
platform = "telegram"
bot_token = "bot-token-123"
agent = "assistant"

[[cron]]
name = "daily"
schedule = "0 0 9 * * *"
agent = "assistant"
message = "Good morning"

[skills]
directory = "skills/"

[[mcp_servers]]
name = "playwright"
command = "npx"
args = ["playwright-mcp"]
"#;
    let config = GatewayConfig::from_toml(toml).unwrap();
    assert_eq!(config.memory.backend, MemoryBackendKind::Sqlite);
    assert_eq!(config.memory.path.as_deref(), Some("data/memory.db"));
    assert_eq!(config.agents.len(), 1);
    assert_eq!(config.agents[0].name.as_str(), "assistant");
    assert_eq!(config.agents[0].tools.len(), 1);
    assert_eq!(config.auth.api_keys.len(), 2);
    assert_eq!(config.channels.len(), 1);
    assert_eq!(config.cron.len(), 1);
    assert_eq!(config.cron[0].schedule, "0 0 9 * * *");
    assert!(config.skills.is_some());
    assert_eq!(config.mcp_servers.len(), 1);
    assert_eq!(config.mcp_servers[0].name.as_str(), "playwright");
    assert!(config.mcp_servers[0].auto_restart);
}

#[test]
fn default_server_config() {
    let toml = r#"
[server]

[llm]
model = "deepseek-chat"
api_key = "key"
"#;
    let config = GatewayConfig::from_toml(toml).unwrap();
    assert_eq!(config.server.host, "127.0.0.1");
    assert_eq!(config.server.port, 3000);
}

#[test]
fn default_memory_config() {
    let toml = r#"
[server]

[llm]
model = "deepseek-chat"
api_key = "key"
"#;
    let config = GatewayConfig::from_toml(toml).unwrap();
    assert_eq!(config.memory.backend, MemoryBackendKind::InMemory);
    assert!(config.memory.path.is_none());
}

#[test]
fn bind_address() {
    let toml = r#"
[server]
host = "0.0.0.0"
port = 8080

[llm]
model = "deepseek-chat"
api_key = "key"
"#;
    let config = GatewayConfig::from_toml(toml).unwrap();
    assert_eq!(config.bind_address(), "0.0.0.0:8080");
}

#[test]
fn env_var_expansion() {
    unsafe { std::env::set_var("TEST_WALRUS_KEY", "expanded-value") };
    let toml = r#"
[server]

[llm]
model = "deepseek-chat"
api_key = "${TEST_WALRUS_KEY}"
"#;
    let config = GatewayConfig::from_toml(toml).unwrap();
    assert_eq!(config.llm.api_key, "expanded-value");
    unsafe { std::env::remove_var("TEST_WALRUS_KEY") };
}

#[test]
fn mcp_server_config() {
    let toml = r#"
[server]

[llm]
model = "deepseek-chat"
api_key = "key"

[[mcp_servers]]
name = "test-server"
command = "test-cmd"
args = ["--flag"]
auto_restart = false

[mcp_servers.env]
KEY = "value"
"#;
    let config = GatewayConfig::from_toml(toml).unwrap();
    assert_eq!(config.mcp_servers.len(), 1);
    let mcp = &config.mcp_servers[0];
    assert_eq!(mcp.name.as_str(), "test-server");
    assert_eq!(mcp.command, "test-cmd");
    assert_eq!(mcp.args, vec!["--flag"]);
    assert!(!mcp.auto_restart);
    assert_eq!(mcp.env.get("KEY").unwrap(), "value");
}
