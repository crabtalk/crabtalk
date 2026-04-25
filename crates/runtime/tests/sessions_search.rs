//! Session search index — direct unit/integration tests on the
//! `SessionIndex` API. Doesn't go through `Runtime` because the index
//! is a pure data structure and we want fast, deterministic coverage
//! of the lexical-recall behavior.

use crabtalk_runtime::sessions::{SearchOptions, SessionIndex};
use wcore::{
    model::{HistoryEntry, Role},
    storage::SessionHandle,
};

fn h(slug: &str) -> SessionHandle {
    SessionHandle::new(slug)
}

fn ensure(index: &mut SessionIndex, handle: &SessionHandle) -> u64 {
    index.ensure_session(
        handle,
        "crab",
        "tester",
        handle.as_str(),
        None,
        "2026-04-25T00:00:00Z",
        "2026-04-25T00:00:00Z",
    )
}

#[test]
fn search_returns_hit_with_handle_and_window() {
    let mut idx = SessionIndex::new();
    let handle = h("crab_tester_1");
    let sid = ensure(&mut idx, &handle);
    idx.insert_message(sid, &HistoryEntry::user("planning the cron refactor"));
    idx.insert_message(
        sid,
        &HistoryEntry::assistant("OK, splitting the daemon.", None, None),
    );

    let hits = idx.search("cron", &SearchOptions::default());
    assert_eq!(hits.len(), 1);
    let hit = &hits[0];
    assert_eq!(&hit.session_handle, &handle);
    assert!(!hit.window.is_empty());
    assert!(hit.window.iter().any(|w| w.snippet.contains("cron")));
}

#[test]
fn search_skips_auto_injected_entries() {
    let mut idx = SessionIndex::new();
    let handle = h("crab_tester_2");
    let sid = ensure(&mut idx, &handle);
    let mut env_block = HistoryEntry::user("crontab schedules");
    env_block.auto_injected = true;
    idx.insert_message(sid, &env_block);
    idx.insert_message(sid, &HistoryEntry::user("unrelated chatter"));

    let hits = idx.search("crontab", &SearchOptions::default());
    assert!(
        hits.is_empty(),
        "auto-injected env block must not be indexed"
    );
}

#[test]
fn agent_filter_excludes_other_agents() {
    let mut idx = SessionIndex::new();
    let other = h("crab_other_1");
    let sid_other = idx.ensure_session(
        &other,
        "crab",
        "other",
        "",
        None,
        "2026-04-25T00:00:00Z",
        "2026-04-25T00:00:00Z",
    );
    idx.insert_message(sid_other, &HistoryEntry::user("matching keyword foobar"));

    let mine = h("crab_tester_3");
    let sid_mine = idx.ensure_session(
        &mine,
        "crab",
        "tester",
        "",
        None,
        "2026-04-25T00:00:00Z",
        "2026-04-25T00:00:00Z",
    );
    idx.insert_message(sid_mine, &HistoryEntry::user("matching keyword foobar"));

    let opts = SearchOptions {
        sender_filter: Some("tester".to_owned()),
        ..SearchOptions::default()
    };

    let hits = idx.search("foobar", &opts);
    assert_eq!(hits.len(), 1);
    assert_eq!(&hits[0].session_handle, &mine);
}

#[test]
fn user_messages_outweigh_assistant_for_same_match() {
    let mut idx = SessionIndex::new();
    let user_session = h("u_user_1");
    let asst_session = h("u_asst_1");
    let su = ensure(&mut idx, &user_session);
    let sa = ensure(&mut idx, &asst_session);
    idx.insert_message(su, &HistoryEntry::user("the deploy pipeline failed"));
    idx.insert_message(
        sa,
        &HistoryEntry::assistant("the deploy pipeline failed", None, None),
    );

    let hits = idx.search("deploy pipeline", &SearchOptions::default());
    assert!(hits.len() >= 2);
    assert_eq!(
        &hits[0].session_handle, &user_session,
        "user role weight should rank above assistant for identical content"
    );
}

#[test]
fn forget_session_removes_postings() {
    let mut idx = SessionIndex::new();
    let handle = h("crab_tester_4");
    let sid = ensure(&mut idx, &handle);
    idx.insert_message(sid, &HistoryEntry::user("zebra crossing"));
    assert_eq!(idx.search("zebra", &SearchOptions::default()).len(), 1);

    idx.forget_session(sid);
    assert!(idx.search("zebra", &SearchOptions::default()).is_empty());
    assert_eq!(idx.session_count(), 0);
    assert_eq!(idx.message_count(), 0);
}

#[test]
fn long_message_snippet_is_truncated() {
    let mut idx = SessionIndex::new();
    let handle = h("crab_tester_5");
    let sid = ensure(&mut idx, &handle);
    let big = "elephant ".repeat(500); // >> MAX_SNIPPET_BYTES
    idx.insert_message(sid, &HistoryEntry::user(big));

    let hits = idx.search("elephant", &SearchOptions::default());
    assert_eq!(hits.len(), 1);
    let item = &hits[0].window[0];
    assert!(item.truncated);
    assert!(item.snippet.len() <= 1024);
}

#[test]
fn ensure_session_is_idempotent_and_refreshes_meta() {
    let mut idx = SessionIndex::new();
    let handle = h("crab_tester_6");
    let id1 = idx.ensure_session(
        &handle,
        "crab",
        "tester",
        "old-title",
        None,
        "2026-04-25T00:00:00Z",
        "2026-04-25T00:00:00Z",
    );
    let id2 = idx.ensure_session(
        &handle,
        "crab",
        "tester",
        "new-title",
        None,
        "2026-04-25T00:00:00Z",
        "2026-04-25T00:00:01Z",
    );
    assert_eq!(id1, id2);
    idx.insert_message(id1, &HistoryEntry::user("anchor message"));

    let hits = idx.search("anchor", &SearchOptions::default());
    assert_eq!(hits[0].title, "new-title");
    assert_eq!(hits[0].updated_at, "2026-04-25T00:00:01Z");
}

#[test]
fn summary_boost_outranks_message_only_match() {
    let mut idx = SessionIndex::new();
    // Both sessions contain a single user message that mentions
    // "rollback" once. One additionally has a compaction summary
    // that mentions rollback — the summary boost should win.
    let plain = h("crab_a_1");
    let plain_id = idx.ensure_session(
        &plain,
        "crab",
        "a",
        "",
        None,
        "2026-04-25T00:00:00Z",
        "2026-04-25T00:00:00Z",
    );
    idx.insert_message(plain_id, &HistoryEntry::user("we tried rollback"));

    let boosted = h("crab_b_1");
    let boosted_id = idx.ensure_session(
        &boosted,
        "crab",
        "b",
        "",
        Some("the rollback recovered the deploy"),
        "2026-04-25T00:00:00Z",
        "2026-04-25T00:00:00Z",
    );
    idx.insert_message(boosted_id, &HistoryEntry::user("we tried rollback"));

    let hits = idx.search("rollback", &SearchOptions::default());
    assert_eq!(hits.len(), 2);
    assert_eq!(
        &hits[0].session_handle, &boosted,
        "session with a summary that contains the query should outrank one without"
    );
    assert!(hits[0].score > hits[1].score);
}

#[test]
fn title_boost_outranks_title_only_message() {
    let mut idx = SessionIndex::new();
    // Two equivalent message hits; one session also has a matching
    // title. Title boost should pull it ahead.
    let plain = h("crab_c_1");
    let plain_id = idx.ensure_session(
        &plain,
        "crab",
        "c",
        "",
        None,
        "2026-04-25T00:00:00Z",
        "2026-04-25T00:00:00Z",
    );
    idx.insert_message(plain_id, &HistoryEntry::user("ranking sanity"));

    let titled = h("crab_d_1");
    let titled_id = idx.ensure_session(
        &titled,
        "crab",
        "d",
        "ranking",
        None,
        "2026-04-25T00:00:00Z",
        "2026-04-25T00:00:00Z",
    );
    idx.insert_message(titled_id, &HistoryEntry::user("ranking sanity"));

    let hits = idx.search("ranking", &SearchOptions::default());
    assert_eq!(hits.len(), 2);
    assert_eq!(&hits[0].session_handle, &titled);
}

#[test]
fn tool_result_content_is_not_searchable() {
    let mut idx = SessionIndex::new();
    let handle = h("crab_tester_priv");
    let sid = ensure(&mut idx, &handle);
    // A tool result containing what looks like a credential. Indexing
    // its content would let any future search query surface it as a
    // free-text hit — secrets must not appear via keyword recall.
    idx.insert_message(
        sid,
        &HistoryEntry::tool(
            "Authorization: Bearer sk-very-secret-token",
            "call-1",
            "shell",
        ),
    );
    let hits = idx.search("Bearer", &SearchOptions::default());
    assert!(
        hits.is_empty(),
        "tool-result content must not be indexed for search"
    );

    // But the function name on a sibling tool-call assistant message
    // is still searchable so "find conversations where shell ran"
    // works.
    use wcore::model::ToolCall;
    let call = ToolCall {
        index: None,
        id: "call-1".into(),
        kind: wcore::model::ToolType::Function,
        function: wcore::model::FunctionCall {
            name: "shell".into(),
            arguments: "Authorization: Bearer sk-very-secret-token".into(),
        },
    };
    idx.insert_message(sid, &HistoryEntry::assistant("", None, Some(&[call])));
    let by_name = idx.search("shell", &SearchOptions::default());
    assert_eq!(by_name.len(), 1, "tool name must still be searchable");
    let by_args = idx.search("Bearer", &SearchOptions::default());
    assert!(
        by_args.is_empty(),
        "tool-call arguments must not be indexed for search"
    );
}

#[test]
fn tool_call_assistant_indexes_function_name_only() {
    use wcore::model::ToolCall;
    let mut idx = SessionIndex::new();
    let handle = h("crab_tester_7");
    let sid = ensure(&mut idx, &handle);
    let call = ToolCall {
        index: None,
        id: "c1".into(),
        kind: wcore::model::ToolType::Function,
        function: wcore::model::FunctionCall {
            name: "shell".into(),
            arguments: r#"{"command": "rebuild_index"}"#.into(),
        },
    };
    idx.insert_message(sid, &HistoryEntry::assistant("", None, Some(&[call])));

    // Function name is indexed — finding "I ran shell" is the
    // primary use case for searching tool-call activity.
    let by_name = idx.search("shell", &SearchOptions::default());
    assert_eq!(by_name.len(), 1);
    let item = &by_name[0].window[0];
    assert!(matches!(item.role, Role::Assistant));
    assert_eq!(item.tool_name.as_deref(), Some("shell"));
    // The display snippet still includes the args so window context
    // is informative — just not searchable.
    assert!(item.snippet.contains("rebuild_index"));

    // Arguments are deliberately not in the BM25 index.
    let by_args = idx.search("rebuild_index", &SearchOptions::default());
    assert!(
        by_args.is_empty(),
        "tool-call arguments must not be indexed for search"
    );
}
