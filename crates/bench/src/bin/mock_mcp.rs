//! Standalone mock MCP server for cross-framework benchmarks.
//!
//! Run with: `cargo run -p crabtalk-bench --bin mock-mcp`

use crabtalk_bench::{mock_mcp, task};

#[tokio::main]
async fn main() {
    let tasks = task::tasks();
    let (addr, _handle) = mock_mcp::start(&tasks).await;
    eprintln!("mock MCP server listening on http://{addr}/mcp");
    eprintln!(
        "tools: {}",
        tasks
            .iter()
            .flat_map(|t| &t.tools)
            .map(|t| t.name)
            .collect::<Vec<_>>()
            .join(", ")
    );
    eprintln!("press ctrl-c to stop");
    tokio::signal::ctrl_c().await.unwrap();
}
