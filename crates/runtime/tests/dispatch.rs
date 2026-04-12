//! Tests for dispatch logic — handler lookup and delegation.

#[tokio::test]
async fn unknown_tool_rejected() {
    let env = ();
    let err = wcore::ToolDispatcher::dispatch(&env, "nonexistent", "{}", "agent", "", None)
        .await
        .unwrap_err();
    assert!(err.contains("tool not registered"));
}
