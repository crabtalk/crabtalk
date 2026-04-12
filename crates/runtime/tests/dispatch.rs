//! Tests for Env dispatch logic — handler lookup and delegation.

use crabtalk_runtime::{Env, Hook};
use std::sync::Arc;
use wcore::{ToolDispatch, ToolFuture, testing::test_schema};

/// A mock hook that handles one tool called "mock_tool".
struct MockHook;

impl Hook for MockHook {
    fn schema(&self) -> Vec<wcore::model::Tool> {
        vec![test_schema("mock_tool")]
    }

    fn dispatch<'a>(&'a self, name: &'a str, _call: ToolDispatch) -> Option<ToolFuture<'a>> {
        if name == "mock_tool" {
            Some(Box::pin(async { Ok("mock ok".to_owned()) }))
        } else {
            None
        }
    }
}

fn test_env() -> Env<()> {
    Env::new((), Arc::new(MockHook))
}

#[tokio::test]
async fn dispatches_to_hook() {
    let env = test_env();
    let result = wcore::ToolDispatcher::dispatch(&env, "mock_tool", "{}", "agent", "", None).await;
    assert_eq!(result.unwrap(), "mock ok");
}

#[tokio::test]
async fn unknown_tool_rejected() {
    let env = test_env();
    let err = wcore::ToolDispatcher::dispatch(&env, "nonexistent", "{}", "agent", "", None)
        .await
        .unwrap_err();
    assert!(err.contains("tool not registered"));
}
