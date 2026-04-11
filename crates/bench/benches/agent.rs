//! Agent loop benchmarks: pure overhead with and without tool dispatch.

use crabllm_core::{FunctionCall, ToolCall};
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use futures_util::StreamExt;
use std::{future::Future, pin::Pin, sync::Arc};
use wcore::{
    AgentBuilder, AgentConfig, ToolDispatcher, ToolFuture,
    model::{HistoryEntry, Model},
    test_utils::test_provider::{TestProvider, text_chunks, tool_chunks},
};

type BoxFut = Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;

struct FnDispatcher<F>(F);

impl<F> ToolDispatcher for FnDispatcher<F>
where
    F: Fn() -> BoxFut + Send + Sync + 'static,
{
    fn dispatch<'a>(
        &'a self,
        _name: &'a str,
        _args: &'a str,
        _agent: &'a str,
        _sender: &'a str,
        _conversation_id: Option<u64>,
    ) -> ToolFuture<'a> {
        (self.0)()
    }
}

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
                    let mut stream =
                        std::pin::pin!(agent.run_stream(&mut history, None, None, None));
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
                let dispatcher: Arc<dyn ToolDispatcher> = Arc::new(FnDispatcher(|| -> BoxFut {
                    Box::pin(async { Ok("ok".to_owned()) })
                }));
                let agent = AgentBuilder::new(Model::new(provider))
                    .config(AgentConfig::new("bench"))
                    .dispatcher(dispatcher)
                    .build();
                let history = vec![HistoryEntry::user("hi")];
                (agent, history)
            },
            |(agent, mut history)| {
                rt.block_on(async {
                    let mut stream =
                        std::pin::pin!(agent.run_stream(&mut history, None, None, None));
                    while stream.next().await.is_some() {}
                });
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(benches, bench_agent_no_tools, bench_agent_with_tools);
criterion_main!(benches);
