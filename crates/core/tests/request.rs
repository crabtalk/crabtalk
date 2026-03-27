//! Tests for Request builder.

use crabtalk_core::model::{Message, Request, Tool, ToolChoice};

#[test]
fn new_sets_model() {
    let req = Request::new("gpt-4o");
    assert_eq!(req.model, "gpt-4o");
    assert!(req.messages.is_empty());
    assert!(req.tools.is_none());
    assert!(req.tool_choice.is_none());
    assert!(!req.think);
}

#[test]
fn with_messages() {
    let msgs = vec![Message::system("sys"), Message::user("hello")];
    let req = Request::new("test").with_messages(msgs);
    assert_eq!(req.messages.len(), 2);
}

#[test]
fn with_tools() {
    let tools = vec![Tool {
        name: "bash".into(),
        description: "run commands".into(),
        parameters: schemars::Schema::default(),
        strict: true,
    }];
    let req = Request::new("test").with_tools(tools);
    assert_eq!(req.tools.as_ref().unwrap().len(), 1);
}

#[test]
fn with_tool_choice() {
    let req = Request::new("test").with_tool_choice(ToolChoice::Required);
    assert!(matches!(req.tool_choice, Some(ToolChoice::Required)));
}

#[test]
fn with_think() {
    let req = Request::new("test").with_think(true);
    assert!(req.think);
}

#[test]
fn builder_chain() {
    let req = Request::new("claude-3")
        .with_messages(vec![Message::user("hi")])
        .with_tools(vec![])
        .with_tool_choice(ToolChoice::Auto)
        .with_think(false);
    assert_eq!(req.model, "claude-3");
    assert_eq!(req.messages.len(), 1);
    assert!(req.tools.unwrap().is_empty());
}
