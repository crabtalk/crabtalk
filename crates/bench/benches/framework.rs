//! Cross-framework benchmark — same tasks, same mock MCP, different agent runtimes.
//!
//! Prerequisites:
//! 1. Mock MCP server: `cargo run -p crabtalk-bench --bin mock-mcp`
//! 2. Local LLM via ollama (fixed model version)
//! 3. All frameworks running and connected to mock MCP + same LLM:
//!    - crabtalk daemon (port 6688)
//!    - OpenClaw (port 18789)
//!    - OpenCode (port 4096, via `opencode serve`)
//!    - Hermes Agent (port 8080)

use crabtalk_bench::{
    gateway::{
        Gateway, crabtalk::CrabtalkGateway, hermes::HermesGateway, openclaw::OpenClawGateway,
        opencode::OpenCodeGateway,
    },
    task::tasks,
};
use criterion::{Criterion, criterion_group, criterion_main};

fn bench_framework(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let gateways: Vec<(&str, Box<dyn Gateway>)> = vec![
        ("crabtalk", Box::new(CrabtalkGateway::new(6688))),
        (
            "openclaw",
            Box::new(OpenClawGateway::new(
                18789,
                std::env::var("OPENCLAW_TOKEN").unwrap_or_default(),
            )),
        ),
        ("opencode", Box::new(OpenCodeGateway::new(4096))),
        ("hermes", Box::new(HermesGateway::new(8080))),
    ];

    for task in tasks() {
        let mut group = c.benchmark_group(task.name);
        // These are real LLM calls — use fewer samples and longer measurement.
        group.sample_size(10);
        group.measurement_time(std::time::Duration::from_secs(30));

        for (name, gw) in &gateways {
            group.bench_function(*name, |b| {
                b.iter(|| gw.run_task(&rt, &task));
            });
        }
        group.finish();
    }
}

criterion_group!(benches, bench_framework);
criterion_main!(benches);
