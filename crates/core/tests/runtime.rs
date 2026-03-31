//! Tests for Runtime — agent registry, session management, and execution.
//!
//! Uses the `()` Hook (no-op) and TestModel.
//!
//! Note: session operations write to ~/.crabtalk/sessions/ via the global
//! SESSIONS_DIR path. This is a known limitation — the LazyLock global
//! cannot be overridden per-test. The files are tiny JSONL and harmless.

use crabtalk_core::{
    AgentConfig, AgentEvent, AgentStopReason, Runtime,
    model::{Choice, FinishReason, StreamChunk, test_model::TestModel},
};
use futures_util::StreamExt;

fn text_chunks(text: &str) -> Vec<StreamChunk> {
    vec![
        StreamChunk::text(text.into()),
        StreamChunk {
            choices: vec![Choice {
                finish_reason: Some(FinishReason::Stop),
                ..Default::default()
            }],
            ..Default::default()
        },
    ]
}

// --- Agent registry ---

#[tokio::test]
async fn add_agent_and_retrieve() {
    let model = TestModel::with_chunks(vec![]);
    let mut runtime = Runtime::new(model, (), None).await;
    runtime.add_agent(AgentConfig::new("crab"));

    assert!(runtime.agent("crab").is_some());
    assert!(runtime.agent("nonexistent").is_none());
    assert!(runtime.get_agent("crab").is_some());
}

#[tokio::test]
async fn agents_returns_all() {
    let model = TestModel::with_chunks(vec![]);
    let mut runtime = Runtime::new(model, (), None).await;
    runtime.add_agent(AgentConfig::new("a"));
    runtime.add_agent(AgentConfig::new("b"));

    let agents = runtime.agents();
    assert_eq!(agents.len(), 2);
}

// --- Tool registry on Runtime ---

#[tokio::test]
async fn register_and_unregister_tool() {
    let model = TestModel::with_chunks(vec![]);
    let mut runtime = Runtime::new(model, (), None).await;
    let tool = crabtalk_core::model::Tool {
        name: "bash".into(),
        description: "run commands".into(),
        parameters: schemars::Schema::default(),
        strict: true,
    };
    runtime.tools.insert(tool);
    assert!(runtime.tools.remove("bash"));
    assert!(!runtime.tools.remove("bash"));
}

// --- Session management ---

#[tokio::test]
async fn create_session_requires_registered_agent() {
    let model = TestModel::with_chunks(vec![]);
    let runtime = Runtime::new(model, (), None).await;
    let err = runtime
        .create_session("nonexistent", "user")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not registered"));
}

#[tokio::test]
async fn create_and_close_session() {
    let model = TestModel::with_chunks(vec![]);
    let mut runtime = Runtime::new(model, (), None).await;
    runtime.add_agent(AgentConfig::new("crab"));

    let id = runtime.create_session("crab", "user").await.unwrap();
    assert!(runtime.session(id).await.is_some());

    assert!(runtime.close_session(id).await);
    assert!(runtime.session(id).await.is_none());
    assert!(!runtime.close_session(id).await);
}

#[tokio::test]
async fn sessions_lists_all() {
    let model = TestModel::with_chunks(vec![]);
    let mut runtime = Runtime::new(model, (), None).await;
    runtime.add_agent(AgentConfig::new("crab"));

    runtime.create_session("crab", "test-a").await.unwrap();
    runtime.create_session("crab", "test-b").await.unwrap();

    let sessions = runtime.sessions().await;
    assert_eq!(sessions.len(), 2);
}

#[tokio::test]
async fn get_or_create_session_returns_existing() {
    let model = TestModel::with_chunks(vec![]);
    let mut runtime = Runtime::new(model, (), None).await;
    runtime.add_agent(AgentConfig::new("crab"));

    let id1 = runtime
        .get_or_create_session("crab", "test-same")
        .await
        .unwrap();
    let id2 = runtime
        .get_or_create_session("crab", "test-same")
        .await
        .unwrap();
    // Should return same in-memory session
    assert_eq!(id1, id2);
}

#[tokio::test]
async fn get_or_create_session_rejects_unknown_agent() {
    let model = TestModel::with_chunks(vec![]);
    let runtime = Runtime::new(model, (), None).await;
    let err = runtime
        .get_or_create_session("ghost", "user")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not registered"));
}

#[tokio::test]
async fn transfer_sessions_moves_all() {
    let model = TestModel::with_chunks(vec![]);
    let mut runtime1 = Runtime::new(model.clone(), (), None).await;
    runtime1.add_agent(AgentConfig::new("crab"));
    let id = runtime1.create_session("crab", "test-xfer").await.unwrap();

    let mut runtime2 = Runtime::new(model, (), None).await;
    runtime2.add_agent(AgentConfig::new("crab"));
    runtime1.transfer_sessions(&mut runtime2).await;

    // Session should exist in runtime2
    assert!(runtime2.session(id).await.is_some());
}

// --- Execution ---

#[tokio::test]
async fn send_to_returns_response() {
    let model = TestModel::with_chunks(vec![text_chunks("hello back")]);
    let mut runtime = Runtime::new(model, (), None).await;
    runtime.add_agent(AgentConfig::new("crab"));

    let session_id = runtime.create_session("crab", "test-send").await.unwrap();
    let response = runtime.send_to(session_id, "hi", "").await.unwrap();

    assert_eq!(response.stop_reason, AgentStopReason::TextResponse);
    assert_eq!(response.final_response.as_deref(), Some("hello back"));
}

#[tokio::test]
async fn send_to_nonexistent_session_errors() {
    let model = TestModel::with_chunks(vec![]);
    let runtime = Runtime::new(model, (), None).await;
    let err = runtime.send_to(999, "hi", "").await.unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[tokio::test]
async fn send_to_appends_to_history() {
    let model = TestModel::with_chunks(vec![
        text_chunks("first reply"),
        text_chunks("second reply"),
    ]);
    let mut runtime = Runtime::new(model, (), None).await;
    runtime.add_agent(AgentConfig::new("crab"));

    let session_id = runtime
        .create_session("crab", "test-history")
        .await
        .unwrap();
    runtime.send_to(session_id, "hello", "").await.unwrap();
    runtime.send_to(session_id, "again", "").await.unwrap();

    let session_mutex = runtime.session(session_id).await.unwrap();
    let session = session_mutex.lock().await;
    // history: user1 + assistant1 + user2 + assistant2
    assert_eq!(session.history.len(), 4);
}

#[tokio::test]
async fn stream_to_yields_correct_content() {
    let model = TestModel::with_chunks(vec![text_chunks("streamed")]);
    let mut runtime = Runtime::new(model, (), None).await;
    runtime.add_agent(AgentConfig::new("crab"));

    let session_id = runtime.create_session("crab", "test-stream").await.unwrap();

    let mut events = Vec::new();
    let mut stream = std::pin::pin!(runtime.stream_to(session_id, "hi", ""));
    while let Some(event) = stream.next().await {
        events.push(event);
    }

    // Verify text content streamed
    let text: String = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::TextDelta(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "streamed");

    // Verify Done event
    if let AgentEvent::Done(resp) = events.last().unwrap() {
        assert_eq!(resp.stop_reason, AgentStopReason::TextResponse);
        assert_eq!(resp.final_response.as_deref(), Some("streamed"));
    } else {
        panic!("last event should be Done");
    }

    // Verify history was persisted
    let session_mutex = runtime.session(session_id).await.unwrap();
    let session = session_mutex.lock().await;
    assert_eq!(session.history.len(), 2); // user + assistant
}

#[tokio::test]
async fn stream_to_nonexistent_session_yields_error() {
    let model = TestModel::with_chunks(vec![]);
    let runtime = Runtime::new(model, (), None).await;

    let mut events = Vec::new();
    let mut stream = std::pin::pin!(runtime.stream_to(999, "hi", ""));
    while let Some(event) = stream.next().await {
        events.push(event);
    }

    if let AgentEvent::Done(resp) = events.last().unwrap() {
        if let AgentStopReason::Error(msg) = &resp.stop_reason {
            assert!(msg.contains("not found"));
        } else {
            panic!("expected Error stop reason");
        }
    } else {
        panic!("expected Done event");
    }
}
