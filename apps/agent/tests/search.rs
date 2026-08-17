//! Session search, end to end against a real SQLite database.

use crabllm_core::anthropic::Message;
use crabtalk_agent::backend::{SqliteStorage, SqliteStore};
use store::{AgentId, HistoryEntry, SearchOptions, interface::Sessions};

async fn open() -> SqliteStore {
    SqliteStorage::memory_store().await.unwrap()
}

async fn seed(
    s: &SqliteStore,
    agent: &AgentId,
    sender: &str,
    msgs: &[&str],
) -> store::SessionHandle {
    let h = s.create_session(agent, sender).await.unwrap();
    let entries: Vec<HistoryEntry> = msgs
        .iter()
        .enumerate()
        .map(|(i, m)| {
            if i % 2 == 0 {
                HistoryEntry::user(*m)
            } else {
                HistoryEntry::from_message(Message::assistant(*m))
            }
        })
        .collect();
    s.append_session_messages(&h, &entries).await.unwrap();
    h
}

#[tokio::test]
async fn finds_a_message_and_ranks_it() {
    let s = open().await;
    let crab = AgentId::default();
    seed(
        &s,
        &crab,
        "me",
        &["the quick brown fox", "unrelated chatter"],
    )
    .await;
    seed(&s, &crab, "me", &["nothing of interest here"]).await;

    let hits = s
        .search_sessions("quick", &SearchOptions::default())
        .await
        .unwrap();
    assert_eq!(hits.len(), 1, "only one session mentions it");
    assert!(hits[0].score > 0.0, "wire scores are bigger-is-better");
    assert!(!hits[0].window.is_empty(), "hit carries its window");
}

#[tokio::test]
async fn window_surrounds_the_match() {
    let s = open().await;
    seed(
        &s,
        &AgentId::default(),
        "me",
        &["one", "two", "needle", "four", "five"],
    )
    .await;
    let hits = s
        .search_sessions("needle", &SearchOptions::default())
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    let idxs: Vec<u32> = hits[0].window.iter().map(|w| w.msg_idx).collect();
    assert!(idxs.contains(&2), "window includes the match: {idxs:?}");
    assert!(idxs.len() > 1, "window includes context: {idxs:?}");
}

#[tokio::test]
async fn filters_by_agent_and_sender() {
    let s = open().await;
    let crab = AgentId::default();
    let other = AgentId::new();
    seed(&s, &crab, "alice", &["shared keyword"]).await;
    seed(&s, &other, "bob", &["shared keyword"]).await;

    let by_agent = SearchOptions {
        agent_filter: Some(crab),
        ..Default::default()
    };
    let hits = s.search_sessions("keyword", &by_agent).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].agent, crab);

    let by_sender = SearchOptions {
        sender_filter: Some("bob".into()),
        ..Default::default()
    };
    let hits = s.search_sessions("keyword", &by_sender).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].sender, "bob");
}

#[tokio::test]
async fn punctuation_is_terms_not_syntax() {
    let s = open().await;
    seed(
        &s,
        &AgentId::default(),
        "me",
        &["call resolve_dirs(config) please"],
    )
    .await;
    // Would be an FTS5 parse error unquoted.
    let hits = s
        .search_sessions("resolve_dirs(config)", &SearchOptions::default())
        .await
        .unwrap();
    assert_eq!(hits.len(), 1, "punctuation must not blow up the query");
}

#[tokio::test]
async fn deleting_a_session_drops_it_from_search() {
    let s = open().await;
    let h = seed(&s, &AgentId::default(), "me", &["distinctive marker"]).await;
    assert_eq!(
        s.search_sessions("distinctive", &SearchOptions::default())
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(s.delete_session(&h).await.unwrap());
    assert!(
        s.search_sessions("distinctive", &SearchOptions::default())
            .await
            .unwrap()
            .is_empty(),
        "FTS5 tables are virtual and do not cascade"
    );
}

#[tokio::test]
async fn empty_query_returns_nothing() {
    let s = open().await;
    seed(&s, &AgentId::default(), "me", &["anything"]).await;
    assert!(
        s.search_sessions("   ", &SearchOptions::default())
            .await
            .unwrap()
            .is_empty()
    );
}
