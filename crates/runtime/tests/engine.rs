//! Tests for Runtime — agent registry, conversation management, and execution.
//!
//! Uses `Env<()>` with InMemoryStorage. Every test gets its own
//! in-memory storage — no shared global state, no filesystem I/O, no node.

use crabtalk_runtime::{Config, Runtime};
use futures_util::StreamExt;
use std::sync::Arc;
use wcore::{
    AgentConfig, AgentEvent, AgentStopReason,
    model::Model,
    testing::{
        InMemoryStorage,
        provider::{TestProvider, text_chunks},
    },
};

struct TestCfg;

impl Config for TestCfg {
    type Storage = InMemoryStorage;
    type Provider = TestProvider;
    type Env = ();
}

fn runtime(provider: TestProvider) -> Runtime<TestCfg> {
    let storage = Arc::new(InMemoryStorage::new());
    let memory = Arc::new(parking_lot::RwLock::new(memory::Memory::new()));
    Runtime::new(
        Model::new(provider),
        Arc::new(()),
        storage,
        memory,
        wcore::ToolRegistry::new(),
    )
}

// --- Agent registry ---

#[tokio::test]
async fn add_agent_and_retrieve() {
    let runtime = runtime(TestProvider::with_chunks(vec![]));
    runtime.add_agent(AgentConfig::new("crab"));

    assert!(runtime.agent("crab").is_some());
    assert!(runtime.agent("nonexistent").is_none());
}

#[tokio::test]
async fn agents_returns_all() {
    let runtime = runtime(TestProvider::with_chunks(vec![]));
    runtime.add_agent(AgentConfig::new("a"));
    runtime.add_agent(AgentConfig::new("b"));

    let agents = runtime.agents();
    assert_eq!(agents.len(), 2);
}

#[tokio::test]
async fn upsert_agent_replaces_existing() {
    let runtime = runtime(TestProvider::with_chunks(vec![]));
    let mut config = AgentConfig::new("crab");
    config.description = "first".to_owned();
    runtime.upsert_agent(config);
    assert_eq!(runtime.agent("crab").unwrap().description, "first");

    let mut replacement = AgentConfig::new("crab");
    replacement.description = "second".to_owned();
    runtime.upsert_agent(replacement);
    assert_eq!(runtime.agent("crab").unwrap().description, "second");
    assert_eq!(runtime.agents().len(), 1);
}

#[tokio::test]
async fn remove_agent_returns_true_when_present() {
    let runtime = runtime(TestProvider::with_chunks(vec![]));
    runtime.add_agent(AgentConfig::new("crab"));
    assert!(runtime.remove_agent("crab"));
    assert!(runtime.agent("crab").is_none());
    assert!(!runtime.remove_agent("crab"));
}

// --- Tool registry on Runtime ---

#[tokio::test]
async fn register_and_unregister_tool() {
    let mut runtime = runtime(TestProvider::with_chunks(vec![]));
    let tool = wcore::model::Tool {
        kind: wcore::model::ToolType::Function,
        function: wcore::model::FunctionDef {
            name: "bash".into(),
            description: Some("run commands".into()),
            parameters: None,
        },
        strict: None,
    };
    runtime.tools.insert(tool);
    assert!(runtime.tools.remove("bash"));
    assert!(!runtime.tools.remove("bash"));
}

// --- Conversation management ---

#[tokio::test]
async fn create_conversation_requires_registered_agent() {
    let runtime = runtime(TestProvider::with_chunks(vec![]));
    let err = runtime
        .create_conversation("nonexistent", "user")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not registered"));
}

#[tokio::test]
async fn create_and_close_conversation() {
    let runtime = runtime(TestProvider::with_chunks(vec![]));
    runtime.add_agent(AgentConfig::new("crab"));

    let id = runtime.create_conversation("crab", "user").await.unwrap();
    assert!(runtime.conversation(id).await.is_some());

    assert!(runtime.close_conversation(id).await);
    assert!(runtime.conversation(id).await.is_none());
    assert!(!runtime.close_conversation(id).await);
}

#[tokio::test]
async fn conversations_lists_all() {
    let runtime = runtime(TestProvider::with_chunks(vec![]));
    runtime.add_agent(AgentConfig::new("crab"));

    runtime.create_conversation("crab", "test-a").await.unwrap();
    runtime.create_conversation("crab", "test-b").await.unwrap();

    let conversations = runtime.conversations().await;
    assert_eq!(conversations.len(), 2);
}

#[tokio::test]
async fn get_or_create_conversation_returns_existing() {
    let runtime = runtime(TestProvider::with_chunks(vec![]));
    runtime.add_agent(AgentConfig::new("crab"));

    let id1 = runtime
        .get_or_create_conversation("crab", "test-same")
        .await
        .unwrap();
    let id2 = runtime
        .get_or_create_conversation("crab", "test-same")
        .await
        .unwrap();
    // Should return same in-memory conversation
    assert_eq!(id1, id2);
}

#[tokio::test]
async fn find_conversation_id_uses_identity_lookup() {
    let runtime = runtime(TestProvider::with_chunks(vec![]));
    runtime.add_agent(AgentConfig::new("crab"));

    let id = runtime
        .create_conversation("crab", "indexed-user")
        .await
        .unwrap();

    assert_eq!(
        runtime.find_conversation_id("crab", "indexed-user").await,
        Some(id)
    );
}

#[tokio::test]
async fn closing_indexed_conversation_falls_back_to_duplicate() {
    let runtime = runtime(TestProvider::with_chunks(vec![]));
    runtime.add_agent(AgentConfig::new("crab"));

    let first = runtime
        .create_conversation("crab", "same-user")
        .await
        .unwrap();
    let second = runtime
        .create_conversation("crab", "same-user")
        .await
        .unwrap();

    assert_eq!(
        runtime.find_conversation_id("crab", "same-user").await,
        Some(first)
    );

    assert!(runtime.close_conversation(first).await);
    assert_eq!(
        runtime.find_conversation_id("crab", "same-user").await,
        Some(second)
    );
}

#[tokio::test]
async fn get_or_create_conversation_rejects_unknown_agent() {
    let runtime = runtime(TestProvider::with_chunks(vec![]));
    let err = runtime
        .get_or_create_conversation("ghost", "user")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not registered"));
}

#[tokio::test]
async fn transfer_conversations_moves_all() {
    let runtime1 = runtime(TestProvider::with_chunks(vec![]));
    runtime1.add_agent(AgentConfig::new("crab"));
    let id = runtime1
        .create_conversation("crab", "test-xfer")
        .await
        .unwrap();

    let mut runtime2 = runtime(TestProvider::with_chunks(vec![]));
    runtime2.add_agent(AgentConfig::new("crab"));
    runtime1.transfer_conversations(&mut runtime2).await;

    // Conversation should exist in runtime2
    assert!(runtime2.conversation(id).await.is_some());
}

// --- Execution ---

#[tokio::test]
async fn send_to_returns_response() {
    let provider = TestProvider::with_chunks(vec![text_chunks("hello back")]);
    let runtime = runtime(provider);
    runtime.add_agent(AgentConfig::new("crab"));

    let conversation_id = runtime
        .create_conversation("crab", "test-send")
        .await
        .unwrap();
    let response = runtime
        .send_to(conversation_id, "hi", "", None)
        .await
        .unwrap();

    assert_eq!(response.stop_reason, AgentStopReason::TextResponse);
    assert_eq!(response.final_response.as_deref(), Some("hello back"));
}

#[tokio::test]
async fn send_to_nonexistent_conversation_errors() {
    let runtime = runtime(TestProvider::with_chunks(vec![]));
    let err = runtime.send_to(999, "hi", "", None).await.unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[tokio::test]
async fn send_to_appends_to_history() {
    let provider = TestProvider::with_chunks(vec![
        text_chunks("first reply"),
        text_chunks("second reply"),
    ]);
    let runtime = runtime(provider);
    runtime.add_agent(AgentConfig::new("crab"));

    let conversation_id = runtime
        .create_conversation("crab", "test-history")
        .await
        .unwrap();
    runtime
        .send_to(conversation_id, "hello", "", None)
        .await
        .unwrap();
    runtime
        .send_to(conversation_id, "again", "", None)
        .await
        .unwrap();

    let conversation_mutex = runtime.conversation(conversation_id).await.unwrap();
    let conversation = conversation_mutex.lock().await;
    // history: user1 + assistant1 + user2 + assistant2
    assert_eq!(conversation.history.len(), 4);
}

#[tokio::test]
async fn stream_to_yields_correct_content() {
    let provider = TestProvider::with_chunks(vec![text_chunks("streamed")]);
    let runtime = runtime(provider);
    runtime.add_agent(AgentConfig::new("crab"));

    let conversation_id = runtime
        .create_conversation("crab", "test-stream")
        .await
        .unwrap();

    let mut events = Vec::new();
    let mut stream = std::pin::pin!(runtime.stream_to(conversation_id, "hi", "", None));
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
    let conversation_mutex = runtime.conversation(conversation_id).await.unwrap();
    let conversation = conversation_mutex.lock().await;
    assert_eq!(conversation.history.len(), 2); // user + assistant
}

#[tokio::test]
async fn resume_prepends_archive_content_from_memory() {
    use memory::{EntryKind, Op};
    use wcore::{model::HistoryEntry, storage::Storage};

    let storage = Arc::new(InMemoryStorage::new());
    let mem = Arc::new(parking_lot::RwLock::new(memory::Memory::new()));
    mem.write()
        .apply(Op::Add {
            name: "archive-test".into(),
            content: "earlier context, compacted".into(),
            aliases: vec![],
            kind: EntryKind::Archive,
        })
        .unwrap();

    let handle = storage.create_session("crab", "tester").unwrap();
    storage
        .append_session_messages(
            &handle,
            &[
                HistoryEntry::user("pre-compact"),
                HistoryEntry::assistant("pre-compact reply", None, None),
            ],
        )
        .unwrap();
    storage
        .append_session_compact(&handle, "archive-test")
        .unwrap();
    storage
        .append_session_messages(&handle, &[HistoryEntry::user("after-compact")])
        .unwrap();

    let runtime = Runtime::<TestCfg>::new(
        Model::new(TestProvider::with_chunks(vec![])),
        Arc::new(()),
        storage,
        mem,
        wcore::ToolRegistry::new(),
    );
    runtime.add_agent(AgentConfig::new("crab"));

    let conv_id = runtime
        .get_or_create_conversation("crab", "tester")
        .await
        .unwrap();
    let conversation = runtime.conversation(conv_id).await.unwrap();
    let conv = conversation.lock().await;

    assert_eq!(conv.history.len(), 2);
    assert_eq!(conv.history[0].text(), "earlier context, compacted");
    assert_eq!(conv.history[1].text(), "after-compact");
}

#[tokio::test]
async fn resume_injects_placeholder_when_archive_missing() {
    use wcore::{model::HistoryEntry, storage::Storage};

    let storage = Arc::new(InMemoryStorage::new());
    // Empty memory — the referenced archive doesn't exist.
    let mem = Arc::new(parking_lot::RwLock::new(memory::Memory::new()));

    let handle = storage.create_session("crab", "tester").unwrap();
    storage
        .append_session_compact(&handle, "archive-gone")
        .unwrap();
    storage
        .append_session_messages(&handle, &[HistoryEntry::user("continuing")])
        .unwrap();

    let runtime = Runtime::<TestCfg>::new(
        Model::new(TestProvider::with_chunks(vec![])),
        Arc::new(()),
        storage,
        mem,
        wcore::ToolRegistry::new(),
    );
    runtime.add_agent(AgentConfig::new("crab"));

    let conv_id = runtime
        .get_or_create_conversation("crab", "tester")
        .await
        .unwrap();
    let conversation = runtime.conversation(conv_id).await.unwrap();
    let conv = conversation.lock().await;

    assert_eq!(conv.history.len(), 2);
    assert!(conv.history[0].text().contains("archive-gone"));
    assert!(conv.history[0].text().contains("unavailable"));
    assert_eq!(conv.history[1].text(), "continuing");
}

#[tokio::test]
async fn stream_to_nonexistent_conversation_yields_error() {
    let runtime = runtime(TestProvider::with_chunks(vec![]));

    let mut events = Vec::new();
    let mut stream = std::pin::pin!(runtime.stream_to(999, "hi", "", None));
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
