//! Agent loop benchmarks: pure overhead with and without tool dispatch.

use crabllm_core::{FunctionCall, ToolCall};
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use futures_util::StreamExt;
use std::pin::pin;
use tokio::sync::mpsc;
use wcore::{
    AgentBuilder, AgentConfig,
    model::{
        HistoryEntry, Model,
        test_provider::{TestProvider, text_chunks, tool_chunks},
    },
};

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

fn bench_agent_no_tools(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    c.bench_function("agent_no_tools", |b| {
        b.iter_batched(
            || {
                let provider = TestProvider::with_chunks(vec![text_chunks("done")]);
                let agent = AgentBuilder::new(Model::new(provider))
                    .config(AgentConfig::new("bench"))
                    .build();
                let history = vec![HistoryEntry::user("hi")];
                (agent, history)
            },
            |(agent, mut history)| {
                rt.block_on(async {
                    let mut stream = pin!(agent.run_stream(&mut history, None, None, None));
                    while stream.next().await.is_some() {}
                });
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_agent_with_tools(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    c.bench_function("agent_with_tools", |b| {
        b.iter_batched(
            || {
                let call = make_tool_call("bash", r#"{"command":"ls"}"#);
                let provider =
                    TestProvider::with_chunks(vec![tool_chunks(vec![call]), text_chunks("done")]);
                let (tool_tx, tool_rx) = mpsc::unbounded_channel();
                let agent = AgentBuilder::new(Model::new(provider))
                    .config(AgentConfig::new("bench"))
                    .tool_tx(tool_tx)
                    .build();
                let history = vec![HistoryEntry::user("hi")];
                (agent, history, tool_rx)
            },
            |(agent, mut history, mut tool_rx)| {
                rt.block_on(async {
                    let handler = tokio::spawn(async move {
                        while let Some(req) = tool_rx.recv().await {
                            let _ = req.reply.send(Ok("ok".into()));
                        }
                    });
                    let mut stream = pin!(agent.run_stream(&mut history, None, None, None));
                    while stream.next().await.is_some() {}
                    handler.abort();
                });
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(benches, bench_agent_no_tools, bench_agent_with_tools);
criterion_main!(benches);
