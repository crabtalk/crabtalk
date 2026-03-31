//! Tests for Message constructors, MessageBuilder, and token estimation.

use crabtalk_core::model::{
    FunctionCall, Message, MessageBuilder, Role, StreamChunk, ToolCall, estimate_tokens,
};

// --- Message constructors ---

#[test]
fn system_message() {
    let msg = Message::system("you are helpful");
    assert_eq!(msg.role, Role::System);
    assert_eq!(msg.content, "you are helpful");
    assert!(msg.tool_calls.is_empty());
}

#[test]
fn user_message() {
    let msg = Message::user("hello");
    assert_eq!(msg.role, Role::User);
    assert_eq!(msg.content, "hello");
    assert!(msg.sender.is_empty());
}

#[test]
fn user_with_sender() {
    let msg = Message::user_with_sender("hello", "tg:123");
    assert_eq!(msg.role, Role::User);
    assert_eq!(msg.sender, "tg:123");
}

#[test]
fn assistant_text_only() {
    let msg = Message::assistant("response", None, None);
    assert_eq!(msg.role, Role::Assistant);
    assert_eq!(msg.content, "response");
    assert!(msg.reasoning_content.is_empty());
    assert!(msg.tool_calls.is_empty());
}

#[test]
fn assistant_with_reasoning() {
    let msg = Message::assistant("answer", Some("thinking...".into()), None);
    assert_eq!(msg.reasoning_content, "thinking...");
}

#[test]
fn assistant_with_tool_calls() {
    let calls = vec![ToolCall {
        id: "call_1".into(),
        index: 0,
        call_type: "function".into(),
        function: FunctionCall {
            name: "bash".into(),
            arguments: r#"{"command":"ls"}"#.into(),
        },
    }];
    let msg = Message::assistant("", None, Some(&calls));
    assert_eq!(msg.tool_calls.len(), 1);
    assert_eq!(msg.tool_calls[0].function.name, "bash");
}

#[test]
fn tool_message() {
    let msg = Message::tool("output", "call_1", "bash");
    assert_eq!(msg.role, Role::Tool);
    assert_eq!(msg.content, "output");
    assert_eq!(msg.tool_call_id, "call_1");
}

// --- Token estimation ---

#[test]
fn estimate_tokens_simple() {
    let msg = Message::user("hello world"); // 11 chars / 4 = 2
    assert_eq!(msg.estimate_tokens(), 2);
}

#[test]
fn estimate_tokens_includes_tool_calls() {
    let calls = vec![ToolCall {
        id: "x".into(),
        index: 0,
        call_type: "function".into(),
        function: FunctionCall {
            name: "bash".into(),                                   // 4 chars
            arguments: r#"{"command":"echo hello world"}"#.into(), // 30 chars
        },
    }];
    let msg = Message::assistant("", None, Some(&calls));
    // (4 + 30) / 4 = 8
    assert_eq!(msg.estimate_tokens(), 8);
}

#[test]
fn estimate_tokens_slice() {
    let msgs = vec![
        Message::user("hello"),                  // 5 / 4 = 1
        Message::assistant("world", None, None), // 5 / 4 = 1
    ];
    assert_eq!(estimate_tokens(&msgs), 2);
}

#[test]
fn estimate_tokens_empty_message_returns_one() {
    let msg = Message::user("");
    assert_eq!(msg.estimate_tokens(), 1); // min 1
}

// --- Serde round-trip ---

#[test]
fn message_serde_roundtrip_user() {
    let msg = Message::user("hello");
    let json = serde_json::to_string(&msg).unwrap();
    let deser: Message = serde_json::from_str(&json).unwrap();
    assert_eq!(deser.role, Role::User);
    assert_eq!(deser.content, "hello");
}

#[test]
fn message_serde_roundtrip_assistant_with_tools() {
    let calls = vec![ToolCall {
        id: "call_1".into(),
        index: 0,
        call_type: "function".into(),
        function: FunctionCall {
            name: "bash".into(),
            arguments: r#"{"cmd":"ls"}"#.into(),
        },
    }];
    let msg = Message::assistant("thinking", Some("reasoning".into()), Some(&calls));
    let json = serde_json::to_string(&msg).unwrap();
    let deser: Message = serde_json::from_str(&json).unwrap();
    assert_eq!(deser.content, "thinking");
    assert_eq!(deser.reasoning_content, "reasoning");
    assert_eq!(deser.tool_calls.len(), 1);
    assert_eq!(deser.tool_calls[0].function.name, "bash");
}

#[test]
fn message_serde_missing_optional_fields() {
    // Minimal JSON — only role present, all skip_serializing_if fields absent.
    let json = r#"{"role":"assistant"}"#;
    let msg: Message = serde_json::from_str(json).unwrap();
    assert_eq!(msg.role, Role::Assistant);
    assert!(msg.content.is_empty());
    assert!(msg.tool_calls.is_empty());
}

#[test]
fn message_serde_tool_message() {
    let msg = Message::tool("result text", "call_42", "bash");
    let json = serde_json::to_string(&msg).unwrap();
    let deser: Message = serde_json::from_str(&json).unwrap();
    assert_eq!(deser.role, Role::Tool);
    assert_eq!(deser.content, "result text");
    assert_eq!(deser.tool_call_id, "call_42");
}

// --- MessageBuilder ---

#[test]
fn builder_accumulates_text() {
    let mut builder = MessageBuilder::new(Role::Assistant);
    builder.accept(&StreamChunk::text("hello ".into()));
    builder.accept(&StreamChunk::text("world".into()));
    let msg = builder.build();
    assert_eq!(msg.content, "hello world");
    assert_eq!(msg.role, Role::Assistant);
}

#[test]
fn builder_accumulates_tool_calls() {
    let mut builder = MessageBuilder::new(Role::Assistant);

    // First chunk: tool name
    let chunk1 = StreamChunk::tool(&[ToolCall {
        id: "call_1".into(),
        index: 0,
        call_type: "function".into(),
        function: FunctionCall {
            name: "bash".into(),
            arguments: String::new(),
        },
    }]);
    builder.accept(&chunk1);

    // Second chunk: arguments
    let chunk2 = StreamChunk::tool(&[ToolCall {
        id: String::new(),
        index: 0,
        call_type: String::new(),
        function: FunctionCall {
            name: String::new(),
            arguments: r#"{"cmd":"ls"}"#.into(),
        },
    }]);
    builder.accept(&chunk2);

    let msg = builder.build();
    assert_eq!(msg.tool_calls.len(), 1);
    assert_eq!(msg.tool_calls[0].id, "call_1");
    assert_eq!(msg.tool_calls[0].function.name, "bash");
    assert_eq!(msg.tool_calls[0].function.arguments, r#"{"cmd":"ls"}"#);
}

#[test]
fn builder_peek_tool_calls() {
    let mut builder = MessageBuilder::new(Role::Assistant);
    let chunk = StreamChunk::tool(&[ToolCall {
        id: "c1".into(),
        index: 0,
        call_type: "function".into(),
        function: FunctionCall {
            name: "recall".into(),
            arguments: String::new(),
        },
    }]);
    builder.accept(&chunk);
    let peeked = builder.peek_tool_calls();
    assert_eq!(peeked.len(), 1);
    assert_eq!(peeked[0].function.name, "recall");
}

#[test]
fn builder_skips_empty_name_in_peek() {
    let mut builder = MessageBuilder::new(Role::Assistant);
    let chunk = StreamChunk::tool(&[ToolCall {
        id: "c1".into(),
        index: 0,
        call_type: "function".into(),
        function: FunctionCall {
            name: String::new(), // no name yet
            arguments: String::new(),
        },
    }]);
    builder.accept(&chunk);
    assert!(builder.peek_tool_calls().is_empty());
}

#[test]
fn builder_accept_returns_has_content() {
    let mut builder = MessageBuilder::new(Role::Assistant);
    let has_content = builder.accept(&StreamChunk::text("hi".into()));
    assert!(has_content);
    let no_content = builder.accept(&StreamChunk::tool(&[]));
    assert!(!no_content);
}

#[test]
fn builder_reasoning_content() {
    use crabtalk_core::model::{Choice, Delta};

    let mut builder = MessageBuilder::new(Role::Assistant);
    let chunk = StreamChunk {
        choices: vec![Choice {
            delta: Delta {
                reasoning_content: Some("let me think".into()),
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    };
    builder.accept(&chunk);
    let msg = builder.build();
    assert_eq!(msg.reasoning_content, "let me think");
}
