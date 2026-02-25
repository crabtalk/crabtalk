//! Everything example — combines custom tools, memory, skills, and teams.
//!
//! Demonstrates the full runtime feature set:
//! 1. Custom tool registration (current_time)
//! 2. Skill injection (concise style)
//! 3. Memory context (user preferences)
//! 4. Team delegation (leader + analyst worker)
//!
//! Requires DEEPSEEK_API_KEY. Run with:
//! ```sh
//! cargo run -p walrus-runtime --example everything
//! ```

mod common;

use walrus_runtime::{Memory, Skill, SkillRegistry, SkillTier, build_team, prelude::*};

#[tokio::main]
async fn main() {
    common::init_tracing();
    let mut runtime = common::build_runtime();

    // 1. Register a custom tool — something the LLM can't do natively.
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

    // 2. Load a skill — modifies system prompt to constrain response style.
    let mut registry = SkillRegistry::new();
    registry.add(
        Skill {
            name: "concise".into(),
            description: "Encourages concise responses".into(),
            license: None,
            compatibility: None,
            metadata: [("tags".into(), "style".into())].into_iter().collect(),
            allowed_tools: vec![],
            body: "Always respond in 2-3 sentences maximum.".into(),
        },
        SkillTier::Bundled,
    );
    runtime.set_skills(registry);

    // 3. Store memory context — affects system prompt via compile_relevant().
    runtime
        .memory()
        .set("preference", "User prefers direct answers with examples.");

    // 4. Build a team: leader delegates to analyst worker.
    let leader = Agent::new("leader")
        .system_prompt("You are a team leader. Delegate research to the analyst.")
        .skill_tag("style")
        .tool("current_time");
    let analyst = Agent::new("analyst")
        .description("Research analyst — answers factual questions.")
        .system_prompt("You are a research analyst. Provide well-reasoned answers.")
        .tool("current_time");

    let leader = build_team(leader, vec![analyst], &mut runtime);
    runtime.add_agent(leader);

    println!("Everything REPL — leader + analyst team, tools, memory, skills");
    println!("(type 'exit' to quit)");
    println!("---");
    common::repl(&mut runtime, "leader").await;
}
