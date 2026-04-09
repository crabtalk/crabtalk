//! Tests for Agent execution — step(), run(), run_stream().

use crabllm_core::{FinishReason, FunctionCall, Role, ToolCall};
use crabtalk_core::{
    AgentBuilder, AgentConfig, AgentEvent, AgentStopReason,
    model::{
        HistoryEntry, Model,
        test_provider::{
            TestProvider, finish_chunk, mixed_chunk, text_chunk, text_chunks, text_response,
            thinking_chunk, tool_chunks, tool_response,
        },
    },
};
use futures_util::StreamExt;
use tokio::sync::mpsc;

// ── test-file-local fixture helpers ──

fn make_tool_call(name: &str, args: &str) -> ToolCall {
    ToolCall {
        index: Some(0),
        id: format!("call_{name}"),
        function: FunctionCall {
            name: name.into(),
            arguments: args.into(),
        },
        ..Default::default()
    }
}

fn build_agent_no_tools(model: TestProvider) -> crabtalk_core::Agent<TestProvider> {
    AgentBuilder::new(Model::new(model))
        .config(AgentConfig::new("test-agent"))
        .build()
}

// --- step() tests ---

#[tokio::test]
async fn step_text_response_appends_to_history() {
    let model = TestProvider::new(vec![text_response("hello world")]);
    let agent = build_agent_no_tools(model);
    let mut history = vec![HistoryEntry::user("hi")];
    let step = agent.step(&mut history, None).await.unwrap();

    assert_eq!(history.len(), 2); // user + assistant
    assert_eq!(*history[1].role(), Role::Assistant);
    assert_eq!(history[1].text(), "hello world");
    assert!(step.tool_calls.is_empty());
    assert!(step.tool_results.is_empty());
}

#[tokio::test]
async fn step_tool_calls_dispatched_and_appended() {
    let calls = vec![make_tool_call("bash", r#"{"command":"ls"}"#)];
    let model = TestProvider::new(vec![tool_response(calls)]);

    let (tool_tx, mut tool_rx) = mpsc::unbounded_channel();
    let agent = AgentBuilder::new(Model::new(model))
        .config(AgentConfig::new("test-agent"))
        .tool_tx(tool_tx)
        .build();

    tokio::spawn(async move {
        while let Some(req) = tool_rx.recv().await {
            let _ = req.reply.send(Ok(format!("result for {}", req.name)));
        }
    });

    let mut history = vec![HistoryEntry::user("run ls")];
    let step = agent.step(&mut history, None).await.unwrap();

    assert_eq!(step.tool_calls.len(), 1);
    assert_eq!(step.tool_calls[0].function.name, "bash");
    assert_eq!(step.tool_results.len(), 1);
    assert_eq!(step.tool_results[0].text(), "result for bash");
    // history: user + assistant(tool_calls) + tool(result)
    assert_eq!(history.len(), 3);
}

#[tokio::test]
async fn step_no_tool_sender_returns_error_message() {
    let calls = vec![make_tool_call("bash", "{}")];
    let model = TestProvider::new(vec![tool_response(calls)]);
    let agent = build_agent_no_tools(model);
    let mut history = vec![HistoryEntry::user("hi")];
    let step = agent.step(&mut history, None).await.unwrap();

    assert_eq!(step.tool_results.len(), 1);
    assert!(
        step.tool_results[0]
            .text()
            .contains("no tool sender configured")
    );
}

#[tokio::test]
async fn step_send_error_propagates() {
    // Empty script — send() will error.
    let model = TestProvider::new(vec![]);
    let agent = build_agent_no_tools(model);
    let mut history = vec![HistoryEntry::user("hi")];
    let result = agent.step(&mut history, None).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn run_stream_surfaces_handler_error_end_to_end() {
    // A handler that returns `Err(...)` must:
    //   1. produce an `AgentEvent::ToolResult` whose `output` is `Err`
    //   2. still land the error text in history so the next model call
    //      sees it (the wire Message has no is_error flag)
    let calls = vec![make_tool_call("bash", "{}")];
    let model = TestProvider::with_chunks(vec![tool_chunks(calls), text_chunks("ack")]);

    let (tool_tx, mut tool_rx) = mpsc::unbounded_channel();
    let agent = AgentBuilder::new(Model::new(model))
        .config(AgentConfig::new("test-agent"))
        .tool_tx(tool_tx)
        .build();

    tokio::spawn(async move {
        while let Some(req) = tool_rx.recv().await {
            let _ = req.reply.send(Err(format!("{} blew up", req.name)));
        }
    });

    let mut history = vec![HistoryEntry::user("go")];
    let mut events: Vec<AgentEvent> = Vec::new();
    let mut stream = std::pin::pin!(agent.run_stream(&mut history, None, None, None));
    while let Some(event) = stream.next().await {
        events.push(event);
    }

    // (1) event carries Err — exactly one ToolResult, output.is_err().
    let tool_results: Vec<&AgentEvent> = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::ToolResult { .. }))
        .collect();
    assert_eq!(tool_results.len(), 1);
    match tool_results[0] {
        AgentEvent::ToolResult { output, .. } => {
            assert_eq!(
                output.as_ref().err().map(String::as_str),
                Some("bash blew up")
            );
        }
        _ => unreachable!(),
    }

    // (2) history preserves the error text for the next model call.
    if let AgentEvent::Done(resp) = events.last().unwrap() {
        let tool_entry = resp.steps[0]
            .tool_results
            .last()
            .expect("step has a tool_result entry");
        assert_eq!(tool_entry.text(), "bash blew up");
    } else {
        panic!("last event should be Done");
    }
}

// --- run_stream() tests ---

#[tokio::test]
async fn run_stream_text_response() {
    let model = TestProvider::with_chunks(vec![text_chunks("hello")]);
    let agent = build_agent_no_tools(model);
    let mut history = vec![HistoryEntry::user("hi")];

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

    let model = TestProvider::with_chunks(vec![tool_chunks(calls), text_chunks("answer")]);

    let (tool_tx, mut tool_rx) = mpsc::unbounded_channel();
    let agent = AgentBuilder::new(Model::new(model))
        .config(AgentConfig::new("test-agent"))
        .tool_tx(tool_tx)
        .build();

    tokio::spawn(async move {
        while let Some(req) = tool_rx.recv().await {
            let _ = req.reply.send(Ok("tool output".into()));
        }
    });

    let mut history = vec![HistoryEntry::user("question")];
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
            index: Some(0),
            id: "call_1".into(),
            function: FunctionCall {
                name: "bash".into(),
                arguments: r#"{"cmd":"ls"}"#.into(),
            },
            ..Default::default()
        },
        ToolCall {
            index: Some(1),
            id: "call_2".into(),
            function: FunctionCall {
                name: "recall".into(),
                arguments: r#"{"query":"x"}"#.into(),
            },
            ..Default::default()
        },
    ];

    let model = TestProvider::with_chunks(vec![tool_chunks(calls), text_chunks("done")]);

    let (tool_tx, mut tool_rx) = mpsc::unbounded_channel();
    let agent = AgentBuilder::new(Model::new(model))
        .config(AgentConfig::new("test-agent"))
        .tool_tx(tool_tx)
        .build();

    tokio::spawn(async move {
        while let Some(req) = tool_rx.recv().await {
            let _ = req.reply.send(Ok(format!("result:{}", req.name)));
        }
    });

    let mut history = vec![HistoryEntry::user("multi")];
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
async fn run_stream_dispatches_tool_calls_concurrently() {
    // Three tool calls with descending sleep durations — completion order
    // (fast, med, slow) is the reverse of call order (slow, med, fast).
    // Sequential would take ~600ms; concurrent dispatch should finish in
    // ~300ms (the slowest tool). Durations are spaced so the ~200ms gap
    // between "concurrent" and the test bound survives CI jitter without
    // approaching the sequential baseline. History entries must still
    // appear in call order; ToolResult events must fire in completion order.
    let calls = vec![
        ToolCall {
            index: Some(0),
            id: "call_slow".into(),
            function: FunctionCall {
                name: "slow".into(),
                arguments: "{}".into(),
            },
            ..Default::default()
        },
        ToolCall {
            index: Some(1),
            id: "call_med".into(),
            function: FunctionCall {
                name: "med".into(),
                arguments: "{}".into(),
            },
            ..Default::default()
        },
        ToolCall {
            index: Some(2),
            id: "call_fast".into(),
            function: FunctionCall {
                name: "fast".into(),
                arguments: "{}".into(),
            },
            ..Default::default()
        },
    ];

    let model = TestProvider::with_chunks(vec![tool_chunks(calls), text_chunks("done")]);

    let (tool_tx, mut tool_rx) = mpsc::unbounded_channel();
    let agent = AgentBuilder::new(Model::new(model))
        .config(AgentConfig::new("test-agent"))
        .tool_tx(tool_tx)
        .build();

    // Per-request: spawn a task so siblings can run in parallel. The agent
    // loop sends all three requests before awaiting any reply, so a serial
    // handler here would defeat the test.
    tokio::spawn(async move {
        while let Some(req) = tool_rx.recv().await {
            tokio::spawn(async move {
                let sleep_ms = match req.name.as_str() {
                    "slow" => 300,
                    "med" => 200,
                    "fast" => 100,
                    _ => 0,
                };
                tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
                let _ = req.reply.send(Ok(format!("result:{}", req.name)));
            });
        }
    });

    let mut history = vec![HistoryEntry::user("multi")];
    let start = std::time::Instant::now();
    let mut events: Vec<AgentEvent> = Vec::new();
    let mut stream = std::pin::pin!(agent.run_stream(&mut history, None, None, None));
    while let Some(event) = stream.next().await {
        events.push(event);
    }
    let elapsed = start.elapsed();

    // 1. Wall clock proves concurrency. Sequential baseline is ~600ms,
    //    concurrent baseline ~300ms, bound at 500ms — 200ms slack above
    //    the concurrent expectation, 100ms below the sequential baseline.
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "expected concurrent dispatch (< 500ms), got {elapsed:?}"
    );

    // 2. History: tool results appended in call order regardless of completion order.
    if let AgentEvent::Done(resp) = events.last().unwrap() {
        let results = &resp.steps[0].tool_results;
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].text(), "result:slow");
        assert_eq!(results[1].text(), "result:med");
        assert_eq!(results[2].text(), "result:fast");
    } else {
        panic!("last event should be Done");
    }

    // 3. Event counts.
    let result_count = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::ToolResult { .. }))
        .count();
    let start_count = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::ToolCallsStart(_)))
        .count();
    let complete_count = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::ToolCallsComplete))
        .count();
    assert_eq!(result_count, 3);
    assert_eq!(start_count, 1);
    assert_eq!(complete_count, 1);

    // 4. ToolResult events fire in completion order: fast → med → slow.
    let result_ids: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ToolResult { call_id, .. } => Some(call_id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        result_ids,
        vec!["call_fast", "call_med", "call_slow"],
        "ToolResult events should fire in completion order"
    );
}

#[tokio::test]
async fn run_stream_max_iterations() {
    let calls = vec![make_tool_call("bash", "{}")];

    let model = TestProvider::with_chunks(vec![
        tool_chunks(calls.clone()),
        tool_chunks(calls.clone()),
        tool_chunks(calls),
    ]);

    let (tool_tx, mut tool_rx) = mpsc::unbounded_channel();
    let mut config = AgentConfig::new("test-agent");
    config.max_iterations = 2;
    let agent = AgentBuilder::new(Model::new(model))
        .config(config)
        .tool_tx(tool_tx)
        .build();

    tokio::spawn(async move {
        while let Some(req) = tool_rx.recv().await {
            let _ = req.reply.send(Ok("ok".into()));
        }
    });

    let mut history = vec![HistoryEntry::user("loop")];
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
    let model = TestProvider::with_chunks(vec![vec![finish_chunk(FinishReason::Stop)]]);
    let agent = build_agent_no_tools(model);
    let mut history = vec![HistoryEntry::user("hi")];

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
    let model = TestProvider::with_chunks(vec![]);
    let agent = build_agent_no_tools(model);
    let mut history = vec![HistoryEntry::user("hi")];

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
        thinking_chunk("thinking..."),
        text_chunk("answer"),
        finish_chunk(FinishReason::Stop),
    ];
    let model = TestProvider::with_chunks(vec![chunks]);
    let agent = build_agent_no_tools(model);
    let mut history = vec![HistoryEntry::user("think")];

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

// --- segment boundary tests ---

/// Reduce a sequence of events to just the boundary/delta markers, dropping
/// the actual content. Useful for asserting bracket structure.
fn boundary_shape(events: &[AgentEvent]) -> Vec<&'static str> {
    events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::TextStart => Some("TextStart"),
            AgentEvent::TextDelta(_) => Some("TextDelta"),
            AgentEvent::TextEnd => Some("TextEnd"),
            AgentEvent::ThinkingStart => Some("ThinkingStart"),
            AgentEvent::ThinkingDelta(_) => Some("ThinkingDelta"),
            AgentEvent::ThinkingEnd => Some("ThinkingEnd"),
            AgentEvent::ToolCallsBegin(_) => Some("ToolCallsBegin"),
            AgentEvent::ToolCallsStart(_) => Some("ToolCallsStart"),
            AgentEvent::ToolCallsComplete => Some("ToolCallsComplete"),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn run_stream_text_segment_is_bracketed() {
    let model = TestProvider::with_chunks(vec![text_chunks("hi")]);
    let agent = build_agent_no_tools(model);
    let mut history = vec![HistoryEntry::user("ping")];

    let mut events: Vec<AgentEvent> = Vec::new();
    let mut stream = std::pin::pin!(agent.run_stream(&mut history, None, None, None));
    while let Some(event) = stream.next().await {
        events.push(event);
    }

    let shape = boundary_shape(&events);
    // text_chunks("hi") emits two char deltas — exactly one TextStart and
    // one TextEnd should bracket all of them.
    let starts = shape.iter().filter(|s| **s == "TextStart").count();
    let ends = shape.iter().filter(|s| **s == "TextEnd").count();
    assert_eq!(starts, 1, "expected exactly one TextStart, got {shape:?}");
    assert_eq!(ends, 1, "expected exactly one TextEnd, got {shape:?}");
    // First boundary marker is TextStart, last is TextEnd.
    assert_eq!(shape.first(), Some(&"TextStart"));
    assert_eq!(shape.last(), Some(&"TextEnd"));
}

#[tokio::test]
async fn run_stream_thinking_then_text_brackets_atomically() {
    // Chunk 1: reasoning only.
    // Chunk 2: text only.
    // The transition from thinking to text MUST emit ThinkingEnd before TextStart.
    let chunks = vec![
        thinking_chunk("plan"),
        text_chunk("answer"),
        finish_chunk(FinishReason::Stop),
    ];
    let model = TestProvider::with_chunks(vec![chunks]);
    let agent = build_agent_no_tools(model);
    let mut history = vec![HistoryEntry::user("think")];

    let mut events: Vec<AgentEvent> = Vec::new();
    let mut stream = std::pin::pin!(agent.run_stream(&mut history, None, None, None));
    while let Some(event) = stream.next().await {
        events.push(event);
    }

    let shape = boundary_shape(&events);
    assert_eq!(
        shape,
        vec![
            "ThinkingStart",
            "ThinkingDelta",
            "ThinkingEnd",
            "TextStart",
            "TextDelta",
            "TextEnd",
        ],
        "expected clean atomic flip from thinking to text",
    );
}

#[tokio::test]
async fn run_stream_chunk_with_both_text_and_reasoning_does_not_overlap() {
    // A single chunk carrying both text and reasoning. The current
    // processing order is text first, then reasoning, so the expected
    // shape is TextStart/TextDelta/TextEnd then ThinkingStart/ThinkingDelta.
    // The key invariant: never two segments open simultaneously.
    let chunks = vec![mixed_chunk("a", "b"), finish_chunk(FinishReason::Stop)];
    let model = TestProvider::with_chunks(vec![chunks]);
    let agent = build_agent_no_tools(model);
    let mut history = vec![HistoryEntry::user("hi")];

    let mut events: Vec<AgentEvent> = Vec::new();
    let mut stream = std::pin::pin!(agent.run_stream(&mut history, None, None, None));
    while let Some(event) = stream.next().await {
        events.push(event);
    }

    // Walk the boundary events and assert at most one segment open at a time.
    let mut open_text = false;
    let mut open_thinking = false;
    for e in &events {
        match e {
            AgentEvent::TextStart => {
                assert!(!open_text, "TextStart while text already open");
                assert!(!open_thinking, "TextStart while thinking still open");
                open_text = true;
            }
            AgentEvent::TextEnd => {
                assert!(open_text, "TextEnd without TextStart");
                open_text = false;
            }
            AgentEvent::ThinkingStart => {
                assert!(!open_thinking, "ThinkingStart while thinking already open");
                assert!(!open_text, "ThinkingStart while text still open");
                open_thinking = true;
            }
            AgentEvent::ThinkingEnd => {
                assert!(open_thinking, "ThinkingEnd without ThinkingStart");
                open_thinking = false;
            }
            _ => {}
        }
    }
    assert!(!open_text, "text segment left open at end");
    assert!(!open_thinking, "thinking segment left open at end");
}

#[tokio::test]
async fn run_stream_text_then_tools_closes_text_before_tools() {
    // Text in first iteration, then tools fired in second iteration.
    let calls = vec![make_tool_call("recall", r#"{"q":"x"}"#)];
    let model = TestProvider::with_chunks(vec![tool_chunks(calls), text_chunks("ok")]);

    let (tool_tx, mut tool_rx) = mpsc::unbounded_channel();
    let agent = AgentBuilder::new(Model::new(model))
        .config(AgentConfig::new("test-agent"))
        .tool_tx(tool_tx)
        .build();
    tokio::spawn(async move {
        while let Some(req) = tool_rx.recv().await {
            let _ = req.reply.send(Ok("out".into()));
        }
    });

    let mut history = vec![HistoryEntry::user("q")];
    let mut events: Vec<AgentEvent> = Vec::new();
    let mut stream = std::pin::pin!(agent.run_stream(&mut history, None, None, None));
    while let Some(event) = stream.next().await {
        events.push(event);
    }

    // Assert: every TextStart has a matching TextEnd before any ToolCallsStart
    // that follows it.
    let shape = boundary_shape(&events);
    let mut text_open = false;
    for marker in &shape {
        match *marker {
            "TextStart" => {
                assert!(!text_open, "nested TextStart in {shape:?}");
                text_open = true;
            }
            "TextEnd" => {
                assert!(text_open, "TextEnd without TextStart in {shape:?}");
                text_open = false;
            }
            "ToolCallsStart" => {
                assert!(
                    !text_open,
                    "ToolCallsStart while text segment open in {shape:?}"
                );
            }
            _ => {}
        }
    }
    assert!(!text_open, "text segment left open at end");
}

// --- run() tests ---

#[tokio::test]
async fn run_forwards_events_through_channel() {
    let model = TestProvider::with_chunks(vec![text_chunks("done")]);
    let agent = build_agent_no_tools(model);
    let mut history = vec![HistoryEntry::user("hi")];
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
