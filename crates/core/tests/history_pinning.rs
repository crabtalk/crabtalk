//! Pinning tests for the new `HistoryEntry` + crabllm-typed `MessageBuilder`.
//!
//! These tests pin behaviors that are otherwise load-bearing but untested:
//!
//! 1. **`content: null` discrimination** — assistant + tool_calls + empty
//!    content must serialize with `content` absent (i.e. `null` on the wire),
//!    because stricter OpenAI-compatible providers (deepseek et al.) reject
//!    `{"role":"assistant","content":null}` *without* tool_calls. Every other
//!    combination must carry an explicit (possibly empty) string.
//!
//! 2. **`MessageBuilder::accept` merge order** — ToolCallDelta arrives with
//!    `Option`-everywhere fields and is keyed on `index`. The rules:
//!    - `delta.id: Some(..)` → overwrite
//!    - `delta.function.name: Some(..)` → overwrite
//!    - `delta.function.arguments: Some(..)` → always append (even empty)
//!
//!    Breaking any of these silently corrupts tool calls mid-stream.

use crabllm_core::{
    ChatCompletionChunk, ChunkChoice, Delta, FunctionCallDelta, Role, ToolCall, ToolCallDelta,
    ToolType,
};
use crabtalk_core::model::HistoryEntry;
use crabtalk_core::model::builder::MessageBuilder;

// --- content: null discrimination ---

#[test]
fn assistant_with_tool_calls_and_empty_content_serializes_content_null() {
    let calls = vec![ToolCall {
        index: Some(0),
        id: "call_1".into(),
        kind: ToolType::Function,
        function: crabllm_core::FunctionCall {
            name: "bash".into(),
            arguments: r#"{"cmd":"ls"}"#.into(),
        },
    }];
    let entry = HistoryEntry::assistant("", None, Some(&calls));
    let json = serde_json::to_value(&entry.message).unwrap();
    // The assistant-with-tool-calls-no-text case must serialize as an
    // explicit `"content": null`, matching the old convert::to_ct_message
    // behavior exactly. OpenAI and stricter providers (deepseek et al.) all
    // accept this shape as long as tool_calls is present.
    assert_eq!(
        json.get("content"),
        Some(&serde_json::Value::Null),
        "expected explicit null content, got: {json}",
    );
    assert!(json.get("tool_calls").is_some());
}

#[test]
fn assistant_without_tool_calls_and_empty_content_serializes_empty_string() {
    let entry = HistoryEntry::assistant("", None, None);
    let json = serde_json::to_value(&entry.message).unwrap();
    // Assistant with no tool calls and no text — content must be an explicit
    // empty string, not null/absent. Otherwise stricter providers reject the
    // message with HTTP 400.
    assert_eq!(
        json.get("content"),
        Some(&serde_json::Value::String(String::new())),
        "expected empty string content, got: {json}",
    );
}

#[test]
fn user_empty_content_serializes_empty_string() {
    let entry = HistoryEntry::user("");
    let json = serde_json::to_value(&entry.message).unwrap();
    assert_eq!(
        json.get("content"),
        Some(&serde_json::Value::String(String::new())),
    );
}

#[test]
fn tool_empty_content_serializes_empty_string() {
    let entry = HistoryEntry::tool("", "call_1", "bash");
    let json = serde_json::to_value(&entry.message).unwrap();
    assert_eq!(
        json.get("content"),
        Some(&serde_json::Value::String(String::new())),
    );
}

#[test]
fn assistant_with_content_and_tool_calls_keeps_content() {
    let calls = vec![ToolCall {
        index: Some(0),
        id: "c1".into(),
        kind: ToolType::Function,
        function: crabllm_core::FunctionCall {
            name: "bash".into(),
            arguments: "{}".into(),
        },
    }];
    let entry = HistoryEntry::assistant("thinking", None, Some(&calls));
    let json = serde_json::to_value(&entry.message).unwrap();
    assert_eq!(
        json.get("content"),
        Some(&serde_json::Value::String("thinking".into())),
    );
}

// --- MessageBuilder merge order (ToolCallDelta semantics) ---

fn delta_chunk(deltas: Vec<ToolCallDelta>) -> ChatCompletionChunk {
    ChatCompletionChunk {
        choices: vec![ChunkChoice {
            delta: Delta {
                tool_calls: Some(deltas),
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn builder_merges_tool_call_across_three_deltas() {
    let mut b = MessageBuilder::new(Role::Assistant);

    // Delta 1: id + name, no args yet.
    b.accept(&delta_chunk(vec![ToolCallDelta {
        index: 0,
        id: Some("call_1".into()),
        kind: Some(ToolType::Function),
        function: Some(FunctionCallDelta {
            name: Some("bash".into()),
            arguments: None,
        }),
    }]));

    // Delta 2: first chunk of args.
    b.accept(&delta_chunk(vec![ToolCallDelta {
        index: 0,
        id: None,
        kind: None,
        function: Some(FunctionCallDelta {
            name: None,
            arguments: Some(r#"{"cmd":""#.into()),
        }),
    }]));

    // Delta 3: second chunk of args.
    b.accept(&delta_chunk(vec![ToolCallDelta {
        index: 0,
        id: None,
        kind: None,
        function: Some(FunctionCallDelta {
            name: None,
            arguments: Some(r#"ls"}"#.into()),
        }),
    }]));

    let msg = b.build();
    let calls = msg.tool_calls.as_ref().expect("tool calls present");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_1");
    assert_eq!(calls[0].function.name, "bash");
    assert_eq!(calls[0].function.arguments, r#"{"cmd":"ls"}"#);
}

#[test]
fn builder_id_overwrites_only_when_present() {
    let mut b = MessageBuilder::new(Role::Assistant);
    b.accept(&delta_chunk(vec![ToolCallDelta {
        index: 0,
        id: Some("original".into()),
        kind: None,
        function: Some(FunctionCallDelta {
            name: Some("x".into()),
            arguments: None,
        }),
    }]));
    // A delta with id: None must not clobber the existing id.
    b.accept(&delta_chunk(vec![ToolCallDelta {
        index: 0,
        id: None,
        kind: None,
        function: Some(FunctionCallDelta {
            name: None,
            arguments: Some("args".into()),
        }),
    }]));
    let msg = b.build();
    let calls = msg.tool_calls.as_ref().unwrap();
    assert_eq!(calls[0].id, "original");
    assert_eq!(calls[0].function.name, "x");
    assert_eq!(calls[0].function.arguments, "args");
}

#[test]
fn builder_name_overwrites_only_when_present() {
    let mut b = MessageBuilder::new(Role::Assistant);
    b.accept(&delta_chunk(vec![ToolCallDelta {
        index: 0,
        id: Some("c1".into()),
        kind: None,
        function: Some(FunctionCallDelta {
            name: Some("original_name".into()),
            arguments: None,
        }),
    }]));
    // function.name: None must not clobber.
    b.accept(&delta_chunk(vec![ToolCallDelta {
        index: 0,
        id: None,
        kind: None,
        function: Some(FunctionCallDelta {
            name: None,
            arguments: Some("{}".into()),
        }),
    }]));
    let msg = b.build();
    let calls = msg.tool_calls.as_ref().unwrap();
    assert_eq!(calls[0].function.name, "original_name");
}

#[test]
fn builder_arguments_always_append_never_overwrite() {
    let mut b = MessageBuilder::new(Role::Assistant);
    b.accept(&delta_chunk(vec![ToolCallDelta {
        index: 0,
        id: Some("c1".into()),
        kind: None,
        function: Some(FunctionCallDelta {
            name: Some("bash".into()),
            arguments: Some("ab".into()),
        }),
    }]));
    // Even with a new name/id, arguments still append — never overwrite.
    b.accept(&delta_chunk(vec![ToolCallDelta {
        index: 0,
        id: Some("c1".into()),
        kind: None,
        function: Some(FunctionCallDelta {
            name: Some("bash".into()),
            arguments: Some("cd".into()),
        }),
    }]));
    let msg = b.build();
    let calls = msg.tool_calls.as_ref().unwrap();
    assert_eq!(calls[0].function.arguments, "abcd");
}

#[test]
fn builder_handles_multiple_tool_calls_by_index() {
    let mut b = MessageBuilder::new(Role::Assistant);
    b.accept(&delta_chunk(vec![
        ToolCallDelta {
            index: 0,
            id: Some("call_a".into()),
            kind: None,
            function: Some(FunctionCallDelta {
                name: Some("bash".into()),
                arguments: Some("{}".into()),
            }),
        },
        ToolCallDelta {
            index: 1,
            id: Some("call_b".into()),
            kind: None,
            function: Some(FunctionCallDelta {
                name: Some("read".into()),
                arguments: Some("{}".into()),
            }),
        },
    ]));
    let msg = b.build();
    let calls = msg.tool_calls.as_ref().unwrap();
    assert_eq!(calls.len(), 2);
    // BTreeMap ordering — index 0 first, then 1.
    assert_eq!(calls[0].id, "call_a");
    assert_eq!(calls[0].function.name, "bash");
    assert_eq!(calls[1].id, "call_b");
    assert_eq!(calls[1].function.name, "read");
}

#[test]
fn builder_empty_content_with_tool_calls_builds_assistant_null_content() {
    let mut b = MessageBuilder::new(Role::Assistant);
    b.accept(&delta_chunk(vec![ToolCallDelta {
        index: 0,
        id: Some("c1".into()),
        kind: None,
        function: Some(FunctionCallDelta {
            name: Some("bash".into()),
            arguments: Some("{}".into()),
        }),
    }]));
    let msg = b.build();
    // Same discrimination as HistoryEntry::assistant — assistant with tool
    // calls and no text → explicit `Some(Value::Null)`, which serializes as
    // `"content": null` on the wire.
    assert_eq!(
        msg.content,
        Some(serde_json::Value::Null),
        "expected explicit null, got {:?}",
        msg.content
    );
    assert!(msg.tool_calls.as_ref().unwrap().len() == 1);
    // Round-trip the exact wire shape through serde to catch regressions.
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json.get("content"), Some(&serde_json::Value::Null));
}

#[test]
fn builder_empty_content_without_tool_calls_builds_empty_string() {
    let b = MessageBuilder::new(Role::Assistant);
    let msg = b.build();
    // Empty assistant with no tool calls → explicit empty string, not None.
    assert_eq!(msg.content, Some(serde_json::Value::String(String::new())),);
    assert!(msg.tool_calls.is_none());
}

#[test]
fn builder_peek_tool_calls_skips_empty_names() {
    // Tool call with no function name yet — peek should not surface it.
    let mut b = MessageBuilder::new(Role::Assistant);
    b.accept(&delta_chunk(vec![ToolCallDelta {
        index: 0,
        id: Some("c1".into()),
        kind: None,
        function: Some(FunctionCallDelta {
            name: None,
            arguments: None,
        }),
    }]));
    assert!(b.peek_tool_calls().is_empty());

    // Same call but now with a non-empty name — peek surfaces it.
    b.accept(&delta_chunk(vec![ToolCallDelta {
        index: 0,
        id: None,
        kind: None,
        function: Some(FunctionCallDelta {
            name: Some("bash".into()),
            arguments: None,
        }),
    }]));
    let peeked = b.peek_tool_calls();
    assert_eq!(peeked.len(), 1);
    assert_eq!(peeked[0].function.name, "bash");
}

#[test]
fn builder_accept_returns_true_only_for_visible_text() {
    // A chunk carrying only a tool-call delta should not count as content.
    let mut b = MessageBuilder::new(Role::Assistant);
    let no_text = b.accept(&delta_chunk(vec![ToolCallDelta {
        index: 0,
        id: Some("c1".into()),
        kind: None,
        function: Some(FunctionCallDelta {
            name: Some("bash".into()),
            arguments: None,
        }),
    }]));
    assert!(!no_text);

    // A chunk with content text should return true.
    let text_chunk = ChatCompletionChunk {
        choices: vec![ChunkChoice {
            delta: Delta {
                content: Some("hi".into()),
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    };
    let has_text = b.accept(&text_chunk);
    assert!(has_text);
}
