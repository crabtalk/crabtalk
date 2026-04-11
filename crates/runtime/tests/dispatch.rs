//! Tests for Env dispatch logic — scope enforcement and handler lookup.

use crabtalk_runtime::{Env, NoHost};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, RwLock},
};
use wcore::{AgentConfig, ToolDispatch, ToolEntry, repos::mem::InMemoryStorage};

fn test_hook() -> Env<NoHost, InMemoryStorage> {
    let storage = Arc::new(InMemoryStorage::new());
    let cwd = PathBuf::from("/test");
    let scopes = Arc::new(RwLock::new(BTreeMap::new()));
    let conversation_cwds = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let pending_asks = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let mut env = Env::new(
        storage,
        cwd,
        NoHost,
        scopes,
        conversation_cwds,
        pending_asks,
    );
    // Register a mock tool for testing.
    env.register_tool(ToolEntry {
        schema: wcore::agent::tool::test_schema("mock_tool"),
        handler: Arc::new(|_call: ToolDispatch| Box::pin(async { Ok("mock ok".to_owned()) })),
        system_prompt: None,
        before_run: None,
    });
    env
}

#[tokio::test]
async fn tool_whitelist_rejects_unlisted() {
    let hook = test_hook();
    let mut config = AgentConfig::new("restricted");
    config.tools = vec!["bash".to_owned()];
    hook.register_scope("restricted".to_owned(), &config);

    let err = hook
        .dispatch_tool("mock_tool", "{}", "restricted", "", None)
        .await
        .unwrap_err();
    assert!(err.contains("tool not available"));
}

#[tokio::test]
async fn empty_whitelist_allows_all() {
    let hook = test_hook();
    let config = AgentConfig::new("open");
    hook.register_scope("open".to_owned(), &config);

    let result = hook
        .dispatch_tool("mock_tool", "{}", "open", "", None)
        .await
        .unwrap();
    assert_eq!(result, "mock ok");
}

#[tokio::test]
async fn unknown_tool_rejected() {
    let hook = test_hook();
    let err = hook
        .dispatch_tool("nonexistent", "{}", "agent", "", None)
        .await
        .unwrap_err();
    assert!(err.contains("tool not registered"));
}
