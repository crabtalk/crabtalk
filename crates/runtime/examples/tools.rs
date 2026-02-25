//! Tools example — interactive REPL with a tool LLMs can't do natively.
//!
//! Registers a `current_time` tool that returns the actual UTC time —
//! something LLMs don't have access to. The REPL lets you ask questions
//! and watch the LLM decide when to call the tool.
//!
//! Requires DEEPSEEK_API_KEY. Run with:
//! ```sh
//! cargo run -p walrus-runtime --example tools
//! ```

mod common;

use walrus_runtime::prelude::*;

#[tokio::main]
async fn main() {
    common::init_tracing();
    let mut runtime = common::build_runtime();

    // current_time: LLMs don't know the current time.
    let time_tool = Tool {
        name: "current_time".into(),
        description: "Returns the current UTC date and time.".into(),
        parameters: serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {}
        }))
        .unwrap(),
        strict: false,
    };
    runtime.register(
        time_tool,
        |_| async move { chrono::Utc::now().to_rfc3339() },
    );

    runtime.add_agent(
        Agent::new("assistant")
            .system_prompt(
                "You are a helpful assistant with access to tools. \
                 Use current_time when the user asks about the current time or date.",
            )
            .tool("current_time")
            .tool("remember"),
    );

    println!("Tools REPL — try asking:");
    println!("  'What time is it?'");
    println!("  'What day of the week is it today?'");
    println!("(type 'exit' to quit)");
    println!("---");
    common::repl(&mut runtime, "assistant").await;
}
