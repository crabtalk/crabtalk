//! Tests for ToolCall::merge.

use crabtalk_core::model::{FunctionCall, ToolCall};

#[test]
fn merge_id() {
    let mut base = ToolCall::default();
    let other = ToolCall {
        id: "call_1".into(),
        ..Default::default()
    };
    base.merge(&other);
    assert_eq!(base.id, "call_1");
}

#[test]
fn merge_skips_empty_id() {
    let mut base = ToolCall {
        id: "original".into(),
        ..Default::default()
    };
    let other = ToolCall::default();
    base.merge(&other);
    assert_eq!(base.id, "original");
}

#[test]
fn merge_function_name() {
    let mut base = ToolCall::default();
    let other = ToolCall {
        function: FunctionCall {
            name: "bash".into(),
            arguments: String::new(),
        },
        ..Default::default()
    };
    base.merge(&other);
    assert_eq!(base.function.name, "bash");
}

#[test]
fn merge_appends_arguments() {
    let mut base = ToolCall {
        function: FunctionCall {
            name: "bash".into(),
            arguments: r#"{"cmd":"#.into(),
        },
        ..Default::default()
    };
    let other = ToolCall {
        function: FunctionCall {
            name: String::new(),
            arguments: r#""ls"}"#.into(),
        },
        ..Default::default()
    };
    base.merge(&other);
    assert_eq!(base.function.arguments, r#"{"cmd":"ls"}"#);
}

#[test]
fn merge_call_type() {
    let mut base = ToolCall::default();
    let other = ToolCall {
        call_type: "function".into(),
        ..Default::default()
    };
    base.merge(&other);
    assert_eq!(base.call_type, "function");
}
