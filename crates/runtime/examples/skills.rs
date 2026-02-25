//! Skills example — side-by-side comparison showing skill behavioral effects.
//!
//! Creates two agents with identical prompts: one with a "concise" skill
//! tag, one without. Sends the same questions to both and prints responses
//! side by side so you can see the skill body modifying LLM behavior.
//!
//! Requires DEEPSEEK_API_KEY. Run with:
//! ```sh
//! cargo run -p walrus-runtime --example skills
//! ```

mod common;

use walrus_runtime::{Skill, SkillRegistry, SkillTier, prelude::*};

#[tokio::main]
async fn main() {
    common::init_tracing();
    let mut runtime = common::build_runtime();

    // Register a "concise" skill that constrains response length.
    let mut registry = SkillRegistry::new();
    registry.add(
        Skill {
            name: "concise".into(),
            description: "Constrains responses to exactly 2 sentences.".into(),
            license: None,
            compatibility: None,
            metadata: [("tags".into(), "style".into())].into_iter().collect(),
            allowed_tools: vec![],
            body: "Always respond in exactly 2 sentences. No exceptions.".into(),
        },
        SkillTier::Bundled,
    );
    runtime.set_skills(registry);

    // Two agents: same base prompt, different skill tags.
    runtime
        .add_agent(Agent::new("default").system_prompt("You are a helpful programming assistant."));
    runtime.add_agent(
        Agent::new("concise")
            .system_prompt("You are a helpful programming assistant.")
            .skill_tag("style"),
    );

    let prompts = [
        "Explain what Rust's ownership system is.",
        "How do I create and use a HashMap in Rust?",
        "What are async/await and why are they useful?",
    ];

    for &prompt in &prompts {
        println!("======================================");
        println!("Question: {prompt}");
        println!("--------------------------------------");

        // Send to default agent (no skill).
        let default_response = runtime
            .send_to("default", Message::user(prompt))
            .await
            .expect("default agent failed");
        println!(
            "\n[default agent]:\n{}",
            default_response.content().cloned().unwrap_or_default()
        );

        // Send to concise agent (with skill).
        let concise_response = runtime
            .send_to("concise", Message::user(prompt))
            .await
            .expect("concise agent failed");
        println!(
            "\n[concise agent (skill: 2 sentences)]:\n{}",
            concise_response.content().cloned().unwrap_or_default()
        );

        // Clear sessions so each question is independent.
        runtime.clear_session("default");
        runtime.clear_session("concise");

        println!();
    }
}
