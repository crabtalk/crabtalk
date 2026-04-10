//! Tests for Env dispatch logic — no daemon, no network, no disk.

use crabtalk_runtime::{Env, NoHost};
use std::path::PathBuf;
use wcore::{AgentConfig, repos::mem::InMemoryRepos};

async fn test_hook() -> Env<NoHost, InMemoryRepos> {
    let repos = InMemoryRepos::new();
    let cwd = PathBuf::from("/test");
    Env::new(repos, cwd, None, NoHost)
}

#[tokio::test]
async fn tool_whitelist_rejects_unlisted() {
    let hook = test_hook().await;
    let mut config = AgentConfig::new("restricted");
    config.tools = vec!["bash".to_owned()];
    hook.register_scope("restricted".to_owned(), &config);

    let err = hook
        .dispatch_tool("recall", "{}", "restricted", "", None)
        .await
        .unwrap_err();
    assert!(err.contains("tool not available"));
}

#[tokio::test]
async fn empty_whitelist_allows_all() {
    let hook = test_hook().await;
    let config = AgentConfig::new("open");
    hook.register_scope("open".to_owned(), &config);

    // `recall` with no memory configured is now an Err — it used to be a
    // plain "memory not available" string. The assertion still confirms
    // the whitelist let it through to the handler.
    let err = hook
        .dispatch_tool("recall", r#"{"query":"test"}"#, "open", "", None)
        .await
        .unwrap_err();
    assert!(err.contains("memory not available"));
}

#[tokio::test]
async fn delegate_member_scope_rejects_unlisted_agent() {
    let hook = test_hook().await;
    let mut config = AgentConfig::new("caller");
    config.members = vec!["agent-a".to_owned()];
    hook.register_scope("caller".to_owned(), &config);

    let args = r#"{"tasks":[{"agent":"agent-b","message":"hello"}]}"#;
    let err = hook
        .dispatch_tool("delegate", args, "caller", "", None)
        .await
        .unwrap_err();
    assert!(err.contains("not in your members list"));
}

#[tokio::test]
async fn delegate_member_scope_allows_listed_agent() {
    let hook = test_hook().await;
    let mut config = AgentConfig::new("caller");
    config.members = vec!["agent-a".to_owned()];
    hook.register_scope("caller".to_owned(), &config);

    let args = r#"{"tasks":[{"agent":"agent-a","message":"hello"}]}"#;
    // Scope passes, so the call reaches `Host::dispatch_delegate`. NoHost's
    // default is `Err("delegate is not available in this runtime mode")` —
    // asserting that exact error proves the scope check was cleared (the
    // scope rejection would have short-circuited earlier with a different
    // message).
    let err = hook
        .dispatch_tool("delegate", args, "caller", "", None)
        .await
        .unwrap_err();
    assert_eq!(err, "delegate is not available in this runtime mode");
}

#[tokio::test]
async fn delegate_empty_tasks() {
    let hook = test_hook().await;
    let err = hook
        .dispatch_tool("delegate", r#"{"tasks":[]}"#, "agent", "", None)
        .await
        .unwrap_err();
    assert!(err.contains("no tasks provided"));
}

#[tokio::test]
async fn no_bridge_ask_user_default() {
    let hook = test_hook().await;
    let err = hook
        .dispatch_tool("ask_user", "{}", "agent", "", None)
        .await
        .unwrap_err();
    assert!(err.contains("not available in this runtime mode"));
}

#[tokio::test]
async fn no_bridge_delegate_default() {
    let hook = test_hook().await;
    let args = r#"{"tasks":[{"agent":"x","message":"hi"}]}"#;
    let err = hook
        .dispatch_tool("delegate", args, "agent", "", None)
        .await
        .unwrap_err();
    assert!(err.contains("not available in this runtime mode"));
}

#[tokio::test]
async fn unknown_tool_rejected() {
    let hook = test_hook().await;
    let err = hook
        .dispatch_tool("nonexistent", "{}", "agent", "", None)
        .await
        .unwrap_err();
    assert!(err.contains("tool not available"));
}

#[tokio::test]
async fn bash_rejected_for_gateway_sender() {
    let hook = test_hook().await;
    let err = hook
        .dispatch_tool(
            "bash",
            r#"{"command":"echo hi"}"#,
            "agent",
            "gateway:telegram",
            None,
        )
        .await
        .unwrap_err();
    assert!(err.contains("only available in the command line interface"));
}
