//! Agent example — minimal streaming REPL.
//!
//! The simplest possible agent: one system prompt, streaming responses.
//! For examples with tools, memory, skills, or MCP, see the other examples:
//!   - `memory`  — memory context affecting responses + remember tool
//!   - `skills`  — side-by-side skill comparison
//!   - `tools`   — custom tool registration + REPL
//!   - `mcp`     — MCP server integration
//!   - `everything` — all features combined with team delegation
//!
//! Requires DEEPSEEK_API_KEY. Run with:
//! ```sh
//! cargo run -p walrus-runtime --example agent
//! ```

mod common;

use walrus_runtime::prelude::*;

#[tokio::main]
async fn main() {
    common::init_tracing();
    let mut runtime = common::build_runtime();

    runtime.add_agent(
        Agent::new("assistant").system_prompt("You are a helpful assistant. Be concise."),
    );

    println!("Agent REPL (type 'exit' to quit)");
    println!("---");
    common::repl(&mut runtime, "assistant").await;
}
