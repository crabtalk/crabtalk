//! Tests for team composition.

use agent::{Agent, InMemory};
use llm::{General, LLM};
use walrus_runtime::{Provider, Runtime, build_team, extract_input, worker_tool};

fn test_provider() -> Provider {
    Provider::DeepSeek(deepseek::DeepSeek::new(llm::Client::new(), "test-key").unwrap())
}

#[test]
fn extract_input_parses_json() {
    let json = r#"{"input": "analyze BTC"}"#;
    assert_eq!(extract_input(json).unwrap(), "analyze BTC");
}

#[test]
fn extract_input_fails_on_missing_field() {
    let json = r#"{"query": "analyze BTC"}"#;
    assert!(extract_input(json).is_err());
}

#[test]
fn extract_input_fails_on_invalid_json() {
    assert!(extract_input("not json").is_err());
}

#[test]
fn worker_tool_builds_tool() {
    let t = worker_tool("analyst", "market analysis");
    assert_eq!(t.name, "analyst");
    assert_eq!(t.description, "market analysis");
    assert!(t.strict);
    let json = serde_json::to_string(&t.parameters).unwrap();
    assert!(json.contains("input"));
}

#[test]
fn build_team_registers_workers_as_tools() {
    let mut rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    let leader = Agent::new("leader")
        .system_prompt("You coordinate.")
        .description("coordinator");
    let analyst = Agent::new("analyst")
        .system_prompt("You analyze markets.")
        .description("Market analysis");
    let writer = Agent::new("writer")
        .system_prompt("You write reports.")
        .description("Report writing");

    let leader = build_team(leader, vec![analyst, writer], &mut rt);

    // Leader should have both workers in its tools list.
    assert!(leader.tools.contains(&"analyst".into()));
    assert!(leader.tools.contains(&"writer".into()));

    // Workers should be resolvable as tools in the runtime.
    let tools = rt.resolve(&["analyst".into(), "writer".into()]);
    assert_eq!(tools.len(), 2);
    let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"analyst"));
    assert!(names.contains(&"writer"));
}

#[test]
fn build_team_adds_worker_agents() {
    let mut rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    let leader = Agent::new("leader")
        .system_prompt("You coordinate.")
        .description("coordinator");
    let analyst = Agent::new("analyst")
        .system_prompt("You analyze markets.")
        .description("Market analysis");

    let _leader = build_team(leader, vec![analyst], &mut rt);

    // Worker agents should be registered in the runtime.
    let agent = rt.agent("analyst");
    assert!(agent.is_some());
    assert_eq!(agent.unwrap().system_prompt, "You analyze markets.");
}

#[tokio::test]
async fn worker_handler_parses_input() {
    let mut rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    let leader = Agent::new("leader")
        .system_prompt("You coordinate.")
        .description("coordinator");
    let analyst = Agent::new("analyst")
        .system_prompt("You analyze markets.")
        .description("Market analysis");

    let _leader = build_team(leader, vec![analyst], &mut rt);

    // Dispatch a tool call with invalid JSON to verify input parsing.
    let calls = vec![llm::ToolCall {
        id: "call_1".into(),
        index: 0,
        call_type: "function".into(),
        function: llm::FunctionCall {
            name: "analyst".into(),
            arguments: "not json".into(),
        },
    }];

    let results = rt.dispatch(&calls).await;
    assert_eq!(results.len(), 1);
    assert!(results[0].content.contains("invalid arguments"));
}

#[test]
fn build_team_worker_tool_descriptions() {
    let mut rt = Runtime::<()>::new(General::default(), test_provider(), InMemory::new());
    let leader = Agent::new("leader")
        .system_prompt("You coordinate.")
        .description("coordinator");
    let analyst = Agent::new("analyst")
        .system_prompt("Analyze.")
        .description("Deep market analysis");

    let _leader = build_team(leader, vec![analyst], &mut rt);

    // The resolved tool should carry the worker's description.
    let tools = rt.resolve(&["analyst".into()]);
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].description, "Deep market analysis");
}
