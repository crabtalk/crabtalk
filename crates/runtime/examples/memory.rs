//! Memory example — interactive REPL showing memory context in action.
//!
//! Pre-seeds user context into memory, then starts a REPL. The LLM
//! references stored facts and can use the `remember` tool to store new
//! ones. Memory state is printed after each exchange.
//!
//! Requires DEEPSEEK_API_KEY. Run with:
//! ```sh
//! cargo run -p walrus-runtime --example memory
//! ```

mod common;

use walrus_runtime::{Memory, prelude::*};

#[tokio::main]
async fn main() {
    common::init_tracing();
    let mut runtime = common::build_runtime();

    // Pre-seed memory with user context.
    runtime.memory().set("user_name", "Alex");
    runtime
        .memory()
        .set("preference", "Prefers concise answers with code examples.");
    runtime
        .memory()
        .set("learning", "Currently learning Rust, focus on async.");

    runtime.add_agent(
        Agent::new("assistant")
            .system_prompt(
                "You are a helpful assistant. Use any stored memory about the user \
                 to personalize your responses. When the user shares new information \
                 about themselves, use the remember tool to store it.",
            )
            .tool("remember"),
    );

    println!("Memory REPL — the assistant knows your stored context.");
    println!("Try: 'What do you know about me?' or tell it something new.");
    println!("(type 'exit' to quit)");
    println!("---");

    // Show initial memory state.
    let entries = runtime.memory().entries();
    println!("[Memory: {} entries]", entries.len());
    for (key, value) in &entries {
        println!("  {key} = {value}");
    }
    println!();

    common::repl_with_memory(&mut runtime, "assistant").await;
}
