//! Tests for Env dispatch logic — no daemon, no network, no disk.

use crabtalk_runtime::{Env, NoBackend, SkillHandler, mcp::McpHandler};
use std::path::PathBuf;
use wcore::AgentConfig;

async fn test_hook() -> Env<NoBackend> {
    let skills = SkillHandler::default();
    let mcp = McpHandler::load(&[]).await;
    let cwd = PathBuf::from("/test");
    Env::new(skills, mcp, cwd, None, NoBackend)
}

#[tokio::test]
async fn tool_whitelist_rejects_unlisted() {
    let mut hook = test_hook().await;
    let mut config = AgentConfig::new("restricted");
    config.tools = vec!["bash".to_owned()];
    hook.register_scope("restricted".to_owned(), &config);

    let result: String = hook
        .dispatch_tool("recall", "{}", "restricted", "", None)
        .await;
    assert!(result.contains("tool not available"));
}

#[tokio::test]
async fn empty_whitelist_allows_all() {
    let mut hook = test_hook().await;
    let config = AgentConfig::new("open");
    hook.register_scope("open".to_owned(), &config);

    let result: String = hook
        .dispatch_tool("recall", r#"{"query":"test"}"#, "open", "", None)
        .await;
    assert!(result.contains("memory not available"));
}

#[tokio::test]
async fn delegate_member_scope_rejects_unlisted_agent() {
    let mut hook = test_hook().await;
    let mut config = AgentConfig::new("caller");
    config.members = vec!["agent-a".to_owned()];
    hook.register_scope("caller".to_owned(), &config);

    let args = r#"{"tasks":[{"agent":"agent-b","message":"hello"}]}"#;
    let result: String = hook
        .dispatch_tool("delegate", args, "caller", "", None)
        .await;
    assert!(result.contains("not in your members list"));
}

#[tokio::test]
async fn delegate_member_scope_allows_listed_agent() {
    let mut hook = test_hook().await;
    let mut config = AgentConfig::new("caller");
    config.members = vec!["agent-a".to_owned()];
    hook.register_scope("caller".to_owned(), &config);

    let args = r#"{"tasks":[{"agent":"agent-a","message":"hello"}]}"#;
    let result: String = hook
        .dispatch_tool("delegate", args, "caller", "", None)
        .await;
    assert!(!result.contains("not in your members list"));
}

#[tokio::test]
async fn delegate_empty_tasks() {
    let hook = test_hook().await;
    let result: String = hook
        .dispatch_tool("delegate", r#"{"tasks":[]}"#, "agent", "", None)
        .await;
    assert!(result.contains("no tasks provided"));
}

#[tokio::test]
async fn no_bridge_ask_user_default() {
    let hook = test_hook().await;
    let result: String = hook
        .dispatch_tool("ask_user", "{}", "agent", "", None)
        .await;
    assert!(result.contains("not available in this runtime mode"));
}

#[tokio::test]
async fn no_bridge_delegate_default() {
    let hook = test_hook().await;
    let args = r#"{"tasks":[{"agent":"x","message":"hi"}]}"#;
    let result: String = hook
        .dispatch_tool("delegate", args, "agent", "", None)
        .await;
    assert!(result.contains("not available in this runtime mode"));
}

#[tokio::test]
async fn unknown_tool_rejected() {
    let hook = test_hook().await;
    let result: String = hook
        .dispatch_tool("nonexistent", "{}", "agent", "", None)
        .await;
    assert!(result.contains("tool not available"));
}

#[tokio::test]
async fn bash_rejected_for_gateway_sender() {
    let hook = test_hook().await;
    let result: String = hook
        .dispatch_tool(
            "bash",
            r#"{"command":"echo hi"}"#,
            "agent",
            "gateway:telegram",
            None,
        )
        .await;
    assert!(result.contains("only available in the command line interface"));
}
