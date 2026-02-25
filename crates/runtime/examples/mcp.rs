//! MCP example — connect to a Playwright MCP server and browse the web via REPL.
//!
//! Connects to `@playwright/mcp` for headless browser automation. The LLM
//! can navigate pages, click elements, fill forms, and read page content.
//!
//! Requires DEEPSEEK_API_KEY and `npx` (Node.js 18+). Run with:
//! ```sh
//! cargo run -p walrus-runtime --example mcp
//! ```

mod common;

use walrus_runtime::{McpBridge, prelude::*};

#[tokio::main]
async fn main() {
    common::init_tracing();
    let mut runtime = common::build_runtime();
    let bridge = McpBridge::new();

    // Connect to Playwright MCP server (headless browser automation).
    let mut cmd = tokio::process::Command::new("npx");
    cmd.args(["@playwright/mcp@latest"]);

    println!("Connecting to Playwright MCP server...");
    bridge
        .connect_stdio(cmd)
        .await
        .expect("failed to connect to Playwright MCP server (is npx/node installed?)");

    // List discovered tools.
    let tools = bridge.tools().await;
    println!("Discovered {} MCP tools:", tools.len());
    for tool in &tools {
        println!("  - {}: {}", tool.name, tool.description);
    }

    // Wire MCP tools into the runtime's tool registry.
    runtime.connect_mcp(bridge);
    runtime
        .register_mcp_tools()
        .await
        .expect("failed to register MCP tools");

    runtime.add_agent(
        Agent::new("assistant")
            .system_prompt(
                "You are a helpful web browsing assistant. Use Playwright tools \
                 to navigate pages, interact with elements, and read page content.",
            )
            .tool("*"),
    );

    println!("\nMCP REPL — try asking:");
    println!("  'Go to https://example.com and tell me what you see'");
    println!("  'Search for Rust programming on Wikipedia'");
    println!("(type 'exit' to quit)");
    println!("---");
    common::repl(&mut runtime, "assistant").await;
}
