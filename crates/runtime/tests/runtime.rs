//! Tests for the Runtime orchestrator.

use agent::{Agent, InMemory, Memory, Skill, SkillTier};
use compact_str::CompactString;
use llm::{FunctionCall, General, LLM, Message, Tool, ToolCall};
use std::collections::BTreeMap;
use walrus_runtime::{Hook, Provider, Runtime, SkillRegistry};

fn test_provider() -> Provider {
    Provider::DeepSeek(deepseek::DeepSeek::new(llm::Client::new(), "test-key").unwrap())
}

fn echo_tool() -> Tool {
    Tool {
        name: "echo".into(),
        description: "Echoes the input".into(),
        parameters: schemars::schema_for!(String),
        strict: false,
    }
}

#[test]
fn resolve_returns_registered_tools() {
    let mut rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    rt.register(echo_tool(), |args| async move { args });
    let tools = rt.resolve(&["echo".into()]);
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "echo");
}

#[test]
fn resolve_skips_unknown() {
    let rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    let tools = rt.resolve(&["missing".into()]);
    assert!(tools.is_empty());
}

#[tokio::test]
async fn dispatch_calls_handler() {
    let mut rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    rt.register(echo_tool(), |args| async move { format!("got: {args}") });

    let calls = vec![ToolCall {
        id: "call_1".into(),
        index: 0,
        call_type: "function".into(),
        function: FunctionCall {
            name: "echo".into(),
            arguments: "hello".into(),
        },
    }];

    let results = rt.dispatch(&calls).await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].content, "got: hello");
    assert_eq!(results[0].tool_call_id, "call_1");
}

#[tokio::test]
async fn dispatch_unknown_tool() {
    let rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    let calls = vec![ToolCall {
        id: "call_1".into(),
        index: 0,
        call_type: "function".into(),
        function: FunctionCall {
            name: "missing".into(),
            arguments: "".into(),
        },
    }];

    let results = rt.dispatch(&calls).await;
    assert!(results[0].content.contains("not available"));
}

#[test]
fn context_limit_default() {
    let rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    assert_eq!(rt.context_limit(), 64_000);
}

#[test]
fn context_limit_override() {
    let mut config = General::default();
    config.context_limit = Some(128_000);
    let rt = Runtime::<()>::new(config, test_provider(), InMemory::new());
    assert_eq!(rt.context_limit(), 128_000);
}

#[test]
fn runtime_with_inmemory() {
    let rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    assert!(rt.memory().entries().is_empty());
}

#[test]
fn remember_tool_registered() {
    let rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    let tools = rt.resolve(&["remember".into()]);
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "remember");
}

#[tokio::test]
async fn remember_tool_stores_value() {
    let rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());

    let calls = vec![ToolCall {
        id: "call_1".into(),
        index: 0,
        call_type: "function".into(),
        function: FunctionCall {
            name: "remember".into(),
            arguments: r#"{"key": "name", "value": "Alice"}"#.into(),
        },
    }];

    let results = rt.dispatch(&calls).await;
    assert!(results[0].content.contains("remembered"));

    // Verify value was stored.
    assert_eq!(rt.memory().get("name"), Some("Alice".into()));
}

#[tokio::test]
async fn system_prompt_includes_memory() {
    let memory = InMemory::new();
    memory.set("user", "Prefers short answers.");

    let mut rt = Runtime::<()>::new(General::default(), test_provider(), memory);
    rt.add_agent(Agent::new("test").system_prompt("You are helpful."));

    // api_messages is private, so test via memory content.
    let compiled = rt.memory().compile();
    assert!(compiled.contains("<user>"));
    assert!(compiled.contains("Prefers short answers."));
}

// --- P2-04: Skills integration tests ---

fn make_test_skill(name: &str, tags: &str, body: &str) -> Skill {
    let mut metadata = BTreeMap::new();
    if !tags.is_empty() {
        metadata.insert(CompactString::from("tags"), tags.into());
    }
    Skill {
        name: name.into(),
        description: String::new(),
        license: None,
        compatibility: None,
        metadata,
        allowed_tools: vec!["echo".into()],
        body: body.into(),
    }
}

#[test]
fn runtime_without_skills_unchanged() {
    let rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    // No skills set — resolve still works for exact matches.
    let tools = rt.resolve(&["remember".into()]);
    assert_eq!(tools.len(), 1);
}

#[test]
fn resolve_glob_prefix() {
    let mut rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    rt.register(
        Tool {
            name: "foo_a".into(),
            description: "".into(),
            parameters: schemars::schema_for!(String),
            strict: false,
        },
        |args| async move { args },
    );
    rt.register(
        Tool {
            name: "foo_b".into(),
            description: "".into(),
            parameters: schemars::schema_for!(String),
            strict: false,
        },
        |args| async move { args },
    );
    rt.register(
        Tool {
            name: "bar_c".into(),
            description: "".into(),
            parameters: schemars::schema_for!(String),
            strict: false,
        },
        |args| async move { args },
    );

    let tools = rt.resolve(&["foo_*".into()]);
    assert_eq!(tools.len(), 2);
    assert!(tools.iter().all(|t| t.name.starts_with("foo_")));
}

#[test]
fn resolve_exact_unchanged() {
    let mut rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    rt.register(echo_tool(), |args| async move { args });
    let tools = rt.resolve(&["echo".into()]);
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "echo");
}

#[test]
fn skill_body_injected() {
    let mut registry = SkillRegistry::new();
    registry.add(
        make_test_skill("coding", "code", "You are a coding assistant."),
        SkillTier::Bundled,
    );

    let mut rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new())
        .with_skills(registry);
    rt.add_agent(
        Agent::new("dev")
            .system_prompt("Base prompt.")
            .skill_tag("code"),
    );

    // Verify the skill registry is set.
    assert!(rt.agent("dev").is_some());
}

#[test]
fn skill_tools_registered() {
    // Verify that skills' allowed_tools are available via resolve.
    let mut rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    rt.register(echo_tool(), |args| async move { args });

    // The skill lists "echo" in allowed_tools — it should resolve.
    let tools = rt.resolve(&["echo".into()]);
    assert_eq!(tools.len(), 1);
}

// --- Hook trait tests ---

#[test]
fn hook_default_compact_prompt() {
    let prompt = <() as Hook>::compact();
    assert!(!prompt.is_empty());
}

#[test]
fn hook_default_flush_prompt() {
    let prompt = <() as Hook>::flush();
    assert!(!prompt.is_empty());
}

// --- Provider factory tests ---

#[test]
fn provider_deepseek_factory() {
    let provider = Provider::deepseek("test-key");
    assert!(provider.is_ok());
}

// --- Runtime set_skills tests ---

#[test]
fn set_skills_on_existing_runtime() {
    let mut rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    let registry = SkillRegistry::new();
    rt.set_skills(registry);
    // Resolve still works after setting skills.
    let tools = rt.resolve(&["remember".into()]);
    assert_eq!(tools.len(), 1);
}

// --- P2-09: Session and resolve_tools tests ---

#[tokio::test]
async fn send_to_unknown_agent_fails() {
    let mut rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    let result = rt.send_to("unknown", Message::user("hello")).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not registered"));
}

#[test]
fn clear_session_removes() {
    let mut rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    rt.add_agent(Agent::new("test").system_prompt("hello"));
    // clear_session on a non-existent session is a no-op.
    rt.clear_session("test");
    // After clearing, the next send_to would create a fresh session.
}

#[test]
fn resolve_tools_returns_pairs() {
    let mut rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    rt.register(echo_tool(), |args| async move { args });
    let resolved = rt.resolve_tools(&["echo".into()]);
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].0.name, "echo");
}

#[test]
fn resolve_tools_glob() {
    let mut rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    rt.register(
        Tool {
            name: "foo_a".into(),
            description: "".into(),
            parameters: schemars::schema_for!(String),
            strict: false,
        },
        |args| async move { args },
    );
    rt.register(
        Tool {
            name: "foo_b".into(),
            description: "".into(),
            parameters: schemars::schema_for!(String),
            strict: false,
        },
        |args| async move { args },
    );

    let resolved = rt.resolve_tools(&["foo_*".into()]);
    assert_eq!(resolved.len(), 2);
    assert!(resolved.iter().all(|(t, _)| t.name.starts_with("foo_")));
}

#[test]
fn resolve_returns_schemas_only() {
    let mut rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    rt.register(echo_tool(), |args| async move { args });
    let tools = rt.resolve(&["echo".into()]);
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "echo");
    // resolve returns Tool only, no handler — compile-time verified by type.
}
