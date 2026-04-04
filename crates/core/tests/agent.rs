//! Tests for Agent execution — step(), run(), run_stream().

use crabtalk_core::{
    AgentBuilder, AgentConfig, AgentEvent, AgentStopReason,
    model::{
        Choice, CompletionMeta, Delta, FinishReason, FunctionCall, Message, Response, Role,
        StreamChunk, ToolCall, Usage, test_model::TestModel,
    },
};
use futures_util::StreamExt;
use tokio::sync::mpsc;

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

fn make_tool_call(name: &str, args: &str) -> ToolCall {
    ToolCall {
        id: format!("call_{name}"),
        index: 0,
        call_type: "function".into(),
        function: FunctionCall {
            name: name.into(),
            arguments: args.into(),
        },
    }
}

fn text_chunks(text: &str) -> Vec<StreamChunk> {
    let mut chunks: Vec<StreamChunk> = text
        .chars()
        .map(|c| StreamChunk::text(c.to_string()))
        .collect();
    chunks.push(StreamChunk {
        choices: vec![Choice {
            finish_reason: Some(FinishReason::Stop),
            ..Default::default()
        }],
        ..Default::default()
    });
    chunks
}

fn tool_chunks(calls: Vec<ToolCall>) -> Vec<StreamChunk> {
    vec![
        StreamChunk::tool(&calls),
        StreamChunk {
            choices: vec![Choice {
                finish_reason: Some(FinishReason::ToolCalls),
                ..Default::default()
            }],
            ..Default::default()
        },
    ]
}

fn build_agent_no_tools(model: TestModel) -> crabtalk_core::Agent<TestModel> {
    AgentBuilder::new(model)
        .config(AgentConfig::new("test-agent"))
        .build()
}

// --- step() tests ---

#[tokio::test]
async fn step_text_response_appends_to_history() {
    let model = TestModel::new(vec![text_response("hello world")]);
    let agent = build_agent_no_tools(model);
    let mut history = vec![Message::user("hi")];
    let step = agent.step(&mut history, None).await.unwrap();

    assert_eq!(history.len(), 2); // user + assistant
    assert_eq!(history[1].role, Role::Assistant);
    assert_eq!(history[1].content, "hello world");
    assert!(step.tool_calls.is_empty());
    assert!(step.tool_results.is_empty());
}

#[tokio::test]
async fn step_tool_calls_dispatched_and_appended() {
    let calls = vec![make_tool_call("bash", r#"{"command":"ls"}"#)];
    let model = TestModel::new(vec![tool_response(calls)]);

    let (tool_tx, mut tool_rx) = mpsc::unbounded_channel();
    let agent = AgentBuilder::new(model)
        .config(AgentConfig::new("test-agent"))
        .tool_tx(tool_tx)
        .build();

    tokio::spawn(async move {
        while let Some(req) = tool_rx.recv().await {
            let _ = req.reply.send(format!("result for {}", req.name));
        }
    });

    let mut history = vec![Message::user("run ls")];
    let step = agent.step(&mut history, None).await.unwrap();

    assert_eq!(step.tool_calls.len(), 1);
    assert_eq!(step.tool_calls[0].function.name, "bash");
    assert_eq!(step.tool_results.len(), 1);
    assert_eq!(step.tool_results[0].content, "result for bash");
    // history: user + assistant(tool_calls) + tool(result)
    assert_eq!(history.len(), 3);
}

#[tokio::test]
async fn step_no_tool_sender_returns_error_message() {
    let calls = vec![make_tool_call("bash", "{}")];
    let model = TestModel::new(vec![tool_response(calls)]);
    let agent = build_agent_no_tools(model);
    let mut history = vec![Message::user("hi")];
    let step = agent.step(&mut history, None).await.unwrap();

    assert_eq!(step.tool_results.len(), 1);
    assert!(
        step.tool_results[0]
            .content
            .contains("no tool sender configured")
    );
}

#[tokio::test]
async fn step_send_error_propagates() {
    // Empty model — send() will error
    let model = TestModel::new(vec![]);
    let agent = build_agent_no_tools(model);
    let mut history = vec![Message::user("hi")];
    let result = agent.step(&mut history, None).await;
    assert!(result.is_err());
}

// --- run_stream() tests ---

#[tokio::test]
async fn run_stream_text_response() {
    let model = TestModel::with_chunks(vec![text_chunks("hello")]);
    let agent = build_agent_no_tools(model);
    let mut history = vec![Message::user("hi")];

    let mut events: Vec<AgentEvent> = Vec::new();
    let mut stream = std::pin::pin!(agent.run_stream(&mut history, None, None, None));
    while let Some(event) = stream.next().await {
        events.push(event);
    }

    let done = events.last().unwrap();
    if let AgentEvent::Done(resp) = done {
        assert_eq!(resp.stop_reason, AgentStopReason::TextResponse);
        assert_eq!(resp.final_response.as_deref(), Some("hello"));
        assert_eq!(resp.iterations, 1);
    } else {
        panic!("last event should be Done");
    }

    let text_deltas: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::TextDelta(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text_deltas.join(""), "hello");
}

#[tokio::test]
async fn run_stream_tool_call_then_text() {
    let calls = vec![make_tool_call("recall", r#"{"query":"test"}"#)];

    let model = TestModel::with_chunks(vec![tool_chunks(calls), text_chunks("answer")]);

    let (tool_tx, mut tool_rx) = mpsc::unbounded_channel();
    let agent = AgentBuilder::new(model)
        .config(AgentConfig::new("test-agent"))
        .tool_tx(tool_tx)
        .build();

    tokio::spawn(async move {
        while let Some(req) = tool_rx.recv().await {
            let _ = req.reply.send("tool output".into());
        }
    });

    let mut history = vec![Message::user("question")];
    let mut events: Vec<AgentEvent> = Vec::new();
    let mut stream = std::pin::pin!(agent.run_stream(&mut history, None, None, None));
    while let Some(event) = stream.next().await {
        events.push(event);
    }

    let done = events.last().unwrap();
    if let AgentEvent::Done(resp) = done {
        assert_eq!(resp.stop_reason, AgentStopReason::TextResponse);
        assert_eq!(resp.iterations, 2);
        assert_eq!(resp.steps.len(), 2);
    } else {
        panic!("last event should be Done");
    }

    // Verify event ordering: ToolCallsBegin before ToolCallsStart before ToolResult
    let mut seen_begin = false;
    let mut seen_start = false;
    let mut seen_result = false;
    let mut seen_complete = false;
    for event in &events {
        match event {
            AgentEvent::ToolCallsBegin(_) => {
                assert!(!seen_start, "Begin should come before Start");
                seen_begin = true;
            }
            AgentEvent::ToolCallsStart(_) => {
                assert!(seen_begin, "Start should come after Begin");
                assert!(!seen_result, "Start should come before Result");
                seen_start = true;
            }
            AgentEvent::ToolResult { .. } => {
                assert!(seen_start, "Result should come after Start");
                seen_result = true;
            }
            AgentEvent::ToolCallsComplete => {
                assert!(seen_result, "Complete should come after Result");
                seen_complete = true;
            }
            _ => {}
        }
    }
    assert!(seen_begin);
    assert!(seen_start);
    assert!(seen_result);
    assert!(seen_complete);
}

#[tokio::test]
async fn run_stream_multiple_tool_calls_in_one_step() {
    let calls = vec![
        ToolCall {
            id: "call_1".into(),
            index: 0,
            call_type: "function".into(),
            function: FunctionCall {
                name: "bash".into(),
                arguments: r#"{"cmd":"ls"}"#.into(),
            },
        },
        ToolCall {
            id: "call_2".into(),
            index: 1,
            call_type: "function".into(),
            function: FunctionCall {
                name: "recall".into(),
                arguments: r#"{"query":"x"}"#.into(),
            },
        },
    ];

    let model = TestModel::with_chunks(vec![tool_chunks(calls), text_chunks("done")]);

    let (tool_tx, mut tool_rx) = mpsc::unbounded_channel();
    let agent = AgentBuilder::new(model)
        .config(AgentConfig::new("test-agent"))
        .tool_tx(tool_tx)
        .build();

    tokio::spawn(async move {
        while let Some(req) = tool_rx.recv().await {
            let _ = req.reply.send(format!("result:{}", req.name));
        }
    });

    let mut history = vec![Message::user("multi")];
    let mut events: Vec<AgentEvent> = Vec::new();
    let mut stream = std::pin::pin!(agent.run_stream(&mut history, None, None, None));
    while let Some(event) = stream.next().await {
        events.push(event);
    }

    // Should have 2 ToolResult events
    let tool_results: Vec<&AgentEvent> = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::ToolResult { .. }))
        .collect();
    assert_eq!(tool_results.len(), 2);

    if let AgentEvent::Done(resp) = events.last().unwrap() {
        assert_eq!(resp.stop_reason, AgentStopReason::TextResponse);
        assert_eq!(resp.steps[0].tool_calls.len(), 2);
        assert_eq!(resp.steps[0].tool_results.len(), 2);
    } else {
        panic!("last event should be Done");
    }
}

#[tokio::test]
async fn run_stream_max_iterations() {
    let calls = vec![make_tool_call("bash", "{}")];

    let model = TestModel::with_chunks(vec![
        tool_chunks(calls.clone()),
        tool_chunks(calls.clone()),
        tool_chunks(calls),
    ]);

    let (tool_tx, mut tool_rx) = mpsc::unbounded_channel();
    let mut config = AgentConfig::new("test-agent");
    config.max_iterations = 2;
    let agent = AgentBuilder::new(model)
        .config(config)
        .tool_tx(tool_tx)
        .build();

    tokio::spawn(async move {
        while let Some(req) = tool_rx.recv().await {
            let _ = req.reply.send("ok".into());
        }
    });

    let mut history = vec![Message::user("loop")];
    let mut events: Vec<AgentEvent> = Vec::new();
    let mut stream = std::pin::pin!(agent.run_stream(&mut history, None, None, None));
    while let Some(event) = stream.next().await {
        events.push(event);
    }

    if let AgentEvent::Done(resp) = events.last().unwrap() {
        assert_eq!(resp.stop_reason, AgentStopReason::MaxIterations);
        assert_eq!(resp.iterations, 2);
    } else {
        panic!("last event should be Done");
    }
}

#[tokio::test]
async fn run_stream_no_content_no_tools_stops_with_no_action() {
    let chunks = vec![StreamChunk {
        choices: vec![Choice {
            finish_reason: Some(FinishReason::Stop),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let model = TestModel::with_chunks(vec![chunks]);
    let agent = build_agent_no_tools(model);
    let mut history = vec![Message::user("hi")];

    let mut events: Vec<AgentEvent> = Vec::new();
    let mut stream = std::pin::pin!(agent.run_stream(&mut history, None, None, None));
    while let Some(event) = stream.next().await {
        events.push(event);
    }

    if let AgentEvent::Done(resp) = events.last().unwrap() {
        assert_eq!(resp.stop_reason, AgentStopReason::NoAction);
    } else {
        panic!("last event should be Done");
    }
}

#[tokio::test]
async fn run_stream_error_in_stream() {
    let model = TestModel::with_chunks(vec![]);
    let agent = build_agent_no_tools(model);
    let mut history = vec![Message::user("hi")];

    let mut events: Vec<AgentEvent> = Vec::new();
    let mut stream = std::pin::pin!(agent.run_stream(&mut history, None, None, None));
    while let Some(event) = stream.next().await {
        events.push(event);
    }

    if let AgentEvent::Done(resp) = events.last().unwrap() {
        assert!(matches!(resp.stop_reason, AgentStopReason::Error(_)));
    } else {
        panic!("last event should be Done");
    }
}

#[tokio::test]
async fn run_stream_thinking_delta() {
    let chunks = vec![
        StreamChunk {
            choices: vec![Choice {
                delta: Delta {
                    reasoning_content: Some("thinking...".into()),
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        },
        StreamChunk::text("answer".into()),
        StreamChunk {
            choices: vec![Choice {
                finish_reason: Some(FinishReason::Stop),
                ..Default::default()
            }],
            ..Default::default()
        },
    ];
    let model = TestModel::with_chunks(vec![chunks]);
    let agent = build_agent_no_tools(model);
    let mut history = vec![Message::user("think")];

    let mut events: Vec<AgentEvent> = Vec::new();
    let mut stream = std::pin::pin!(agent.run_stream(&mut history, None, None, None));
    while let Some(event) = stream.next().await {
        events.push(event);
    }

    let thinking: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ThinkingDelta(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(thinking, vec!["thinking..."]);
}

// --- run() tests ---

#[tokio::test]
async fn run_forwards_events_through_channel() {
    let model = TestModel::with_chunks(vec![text_chunks("done")]);
    let agent = build_agent_no_tools(model);
    let mut history = vec![Message::user("hi")];
    let (tx, mut rx) = mpsc::unbounded_channel();

    let response = agent.run(&mut history, tx, None, None).await;
    assert_eq!(response.stop_reason, AgentStopReason::TextResponse);
    assert_eq!(response.final_response.as_deref(), Some("done"));

    // Verify events were actually forwarded through the channel
    let mut event_count = 0;
    let mut has_done = false;
    while let Ok(event) = rx.try_recv() {
        event_count += 1;
        if matches!(event, AgentEvent::Done(_)) {
            has_done = true;
        }
    }
    assert!(event_count > 0, "events should have been sent");
    assert!(has_done, "Done event should have been sent");
}
