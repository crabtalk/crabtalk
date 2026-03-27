//! Tests for Response accessors and StreamChunk constructors.

use crabtalk_core::model::{
    Choice, CompletionMeta, Delta, FinishReason, FunctionCall, Response, Role, StreamChunk,
    ToolCall, Usage,
};

fn text_response(content: &str) -> Response {
    Response {
        meta: CompletionMeta::default(),
        choices: vec![Choice {
            index: 0,
            delta: Delta {
                role: Some(Role::Assistant),
                content: Some(content.into()),
                reasoning_content: None,
                tool_calls: None,
            },
            finish_reason: Some(FinishReason::Stop),
            logprobs: None,
        }],
        usage: Usage::default(),
    }
}

fn tool_response(calls: Vec<ToolCall>) -> Response {
    Response {
        meta: CompletionMeta::default(),
        choices: vec![Choice {
            index: 0,
            delta: Delta {
                role: Some(Role::Assistant),
                content: None,
                reasoning_content: None,
                tool_calls: Some(calls),
            },
            finish_reason: Some(FinishReason::ToolCalls),
            logprobs: None,
        }],
        usage: Usage::default(),
    }
}

// --- Response accessors ---

#[test]
fn content_extracts_text() {
    let resp = text_response("hello");
    assert_eq!(resp.content().unwrap(), "hello");
}

#[test]
fn content_none_when_no_choices() {
    let resp = Response {
        meta: CompletionMeta::default(),
        choices: vec![],
        usage: Usage::default(),
    };
    assert!(resp.content().is_none());
}

#[test]
fn tool_calls_extracts() {
    let calls = vec![ToolCall {
        id: "c1".into(),
        index: 0,
        call_type: "function".into(),
        function: FunctionCall {
            name: "bash".into(),
            arguments: "{}".into(),
        },
    }];
    let resp = tool_response(calls);
    let tc = resp.tool_calls().unwrap();
    assert_eq!(tc.len(), 1);
    assert_eq!(tc[0].function.name, "bash");
}

#[test]
fn reason_extracts() {
    let resp = text_response("hi");
    assert_eq!(*resp.reason().unwrap(), FinishReason::Stop);
}

#[test]
fn message_builds_assistant_message() {
    let resp = text_response("world");
    let msg = resp.message().unwrap();
    assert_eq!(msg.role, Role::Assistant);
    assert_eq!(msg.content, "world");
}

#[test]
fn reasoning_extracts() {
    let resp = Response {
        meta: CompletionMeta::default(),
        choices: vec![Choice {
            index: 0,
            delta: Delta {
                role: Some(Role::Assistant),
                content: Some("answer".into()),
                reasoning_content: Some("thinking".into()),
                tool_calls: None,
            },
            finish_reason: None,
            logprobs: None,
        }],
        usage: Usage::default(),
    };
    assert_eq!(resp.reasoning().unwrap(), "thinking");
}

// --- StreamChunk constructors ---

#[test]
fn chunk_text() {
    let chunk = StreamChunk::text("hello".into());
    assert_eq!(chunk.content().unwrap(), "hello");
    assert!(chunk.tool_calls().is_none());
}

#[test]
fn chunk_text_empty_is_none() {
    let chunk = StreamChunk::text(String::new());
    assert!(chunk.content().is_none());
}

#[test]
fn chunk_tool() {
    let calls = vec![ToolCall {
        id: "c1".into(),
        index: 0,
        call_type: "function".into(),
        function: FunctionCall {
            name: "bash".into(),
            arguments: "{}".into(),
        },
    }];
    let chunk = StreamChunk::tool(&calls);
    assert_eq!(chunk.tool_calls().unwrap().len(), 1);
    assert!(chunk.content().is_none());
}

#[test]
fn chunk_separator() {
    let chunk = StreamChunk::separator();
    // separator is "\n" which is not empty but also tests the path
    assert!(chunk.content().is_some());
}

#[test]
fn chunk_reason() {
    let chunk = StreamChunk {
        choices: vec![Choice {
            finish_reason: Some(FinishReason::Stop),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_eq!(*chunk.reason().unwrap(), FinishReason::Stop);
}
