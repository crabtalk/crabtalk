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
        Gateway, check_reachable, crabtalk::CrabtalkGateway, hermes::HermesGateway,
        openclaw::OpenClawGateway, opencode::OpenCodeGateway,
    },
    task::tasks,
};
use criterion::{Criterion, criterion_group, criterion_main};

fn env_port(var: &str, default: u16) -> u16 {
    std::env::var(var)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn bench_framework(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let all_gateways: Vec<(&str, u16, Box<dyn Gateway>)> = vec![
        {
            let port = env_port("CRABTALK_PORT", 6688);
            ("crabtalk", port, Box::new(CrabtalkGateway::new(port)))
        },
        {
            let port = env_port("OPENCLAW_PORT", 18789);
            let token = std::env::var("OPENCLAW_TOKEN").unwrap_or_default();
            (
                "openclaw",
                port,
                Box::new(OpenClawGateway::new(port, token)),
            )
        },
        {
            let port = env_port("OPENCODE_PORT", 4096);
            ("opencode", port, Box::new(OpenCodeGateway::new(port)))
        },
        {
            let port = env_port("HERMES_PORT", 8080);
            ("hermes", port, Box::new(HermesGateway::new(port)))
        },
    ];

    // Skip frameworks that aren't running.
    let gateways: Vec<_> = all_gateways
        .into_iter()
        .filter(|(name, port, _)| {
            if check_reachable(*port) {
                true
            } else {
                eprintln!("SKIP {name}: not reachable on port {port}");
                false
            }
        })
        .collect();

    if gateways.is_empty() {
        eprintln!("no frameworks available — nothing to benchmark");
        return;
    }

    for task in tasks() {
        let mut group = c.benchmark_group(task.name);
        // These are real LLM calls — use fewer samples and longer measurement.
        group.sample_size(10);
        group.measurement_time(std::time::Duration::from_secs(30));

        for (name, _, gw) in &gateways {
            group.bench_function(*name, |b| {
                b.iter(|| gw.run_task(&rt, &task));
            });
        }
        group.finish();
    }
}

criterion_group!(benches, bench_framework);
criterion_main!(benches);
