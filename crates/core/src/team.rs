//! Team trait: a leader agent with typed workers.
//!
//! Developers implement [`Team`] on their own struct, holding
//! concrete agent types as fields. No `dyn`, no type erasure.
//!
//! # Example
//!
//! ```rust,ignore
//! use ccore::{Team, Agent, Tool};
//!
//! #[derive(Clone)]
//! struct MyTeam { leader: MyLeader, analyst: Analyst }
//!
//! impl Team for MyTeam {
//!     type Leader = MyLeader;
//!     fn leader(&self) -> &MyLeader { &self.leader }
//!     fn workers(&self) -> Vec<Tool> { vec![/* ... */] }
//!     async fn call(&self, name: &str, input: String) -> anyhow::Result<String> {
//!         // route to worker by name
//!         todo!()
//!     }
//! }
//! ```

use crate::{Agent, Message, StreamChunk, Tool, ToolCall};
use anyhow::Result;

/// A team of agents: one leader + typed workers.
///
/// Implement this trait on your own struct. The struct holds
/// concrete agent types as fields — no `dyn`, no generics.
///
/// `Team` has a blanket [`Agent`] implementation, so any team
/// can be used directly with [`Chat`](crate::Chat).
pub trait Team: Clone {
    /// The leader agent type.
    type Leader: Agent<Chunk = StreamChunk>;

    /// The leader agent.
    fn leader(&self) -> &Self::Leader;

    /// Tool definitions for all workers.
    fn workers(&self) -> Vec<Tool>;

    /// Dispatch a task to a worker by name.
    fn call(&self, name: &str, input: String) -> impl Future<Output = Result<String>>;
}

impl<T: Team> Agent for T {
    type Chunk = StreamChunk;

    fn name(&self) -> &str {
        self.leader().name()
    }

    fn description(&self) -> &str {
        self.leader().description()
    }

    fn system_prompt(&self) -> String {
        self.leader().system_prompt()
    }

    fn tools(&self) -> Vec<Tool> {
        let mut tools = self.leader().tools();
        tools.extend(self.workers());
        tools
    }

    fn compact(&self, messages: Vec<Message>) -> Vec<Message> {
        self.leader().compact(messages)
    }

    async fn dispatch(&self, tools: &[ToolCall]) -> Vec<Message> {
        let worker_names: Vec<String> = self.workers().into_iter().map(|t| t.name).collect();
        let mut results = Vec::new();
        let mut regular = Vec::new();

        for call in tools {
            if worker_names.contains(&call.function.name) {
                match crate::team::extract_input(&call.function.arguments) {
                    Ok(input) => match self.call(&call.function.name, input).await {
                        Ok(output) => {
                            results.push(Message::tool(output, &call.id));
                        }
                        Err(e) => {
                            results.push(Message::tool(format!("worker error: {e}"), &call.id));
                        }
                    },
                    Err(e) => {
                        results.push(Message::tool(
                            format!("failed to parse arguments: {e}"),
                            &call.id,
                        ));
                    }
                }
            } else {
                regular.push(call.clone());
            }
        }

        if !regular.is_empty() {
            results.extend(self.leader().dispatch(&regular).await);
        }

        results
    }

    async fn chunk(&self, chunk: &StreamChunk) -> Result<Self::Chunk> {
        self.leader().chunk(chunk).await
    }
}

/// Extract the `input` field from tool call arguments JSON.
pub fn extract_input(arguments: &str) -> Result<String> {
    let parsed: serde_json::Value = serde_json::from_str(arguments)?;
    parsed
        .get("input")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("missing 'input' field in arguments"))
}

/// Build a tool definition for a worker agent.
///
/// Uses a standard `{ input: string }` schema so the leader
/// can delegate tasks with a single text field.
pub fn tool(name: impl Into<String>, description: impl Into<String>) -> Tool {
    Tool {
        name: name.into(),
        description: description.into(),
        parameters: default_input_schema(),
        strict: true,
    }
}

/// Default input schema for agent-as-tool calls.
#[derive(schemars::JsonSchema, serde::Deserialize)]
#[allow(dead_code)]
struct DefaultInput {
    /// The task or question to delegate to this agent.
    input: String,
}

fn default_input_schema() -> schemars::Schema {
    schemars::schema_for!(DefaultInput)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_input_parses_json() {
        let json = r#"{"input": "analyze BTC"}"#;
        assert_eq!(extract_input(json).unwrap(), "analyze BTC");
    }

    #[test]
    fn extract_input_fails_on_missing_field() {
        let json = r#"{"query": "analyze BTC"}"#;
        assert!(extract_input(json).is_err());
    }

    #[test]
    fn extract_input_fails_on_invalid_json() {
        assert!(extract_input("not json").is_err());
    }

    #[test]
    fn tool_helper_builds_tool() {
        let t = tool("analyst", "market analysis");
        assert_eq!(t.name, "analyst");
        assert_eq!(t.description, "market analysis");
        assert!(t.strict);
        let json = serde_json::to_string(&t.parameters).unwrap();
        assert!(json.contains("input"));
    }

    #[test]
    fn team_agent_tools() {
        #[derive(Clone)]
        struct Leader;

        impl Agent for Leader {
            type Chunk = StreamChunk;
            fn system_prompt(&self) -> String {
                "I am the leader".into()
            }
            fn tools(&self) -> Vec<Tool> {
                vec![tool("leader_tool", "leader's own tool")]
            }
            async fn chunk(&self, chunk: &StreamChunk) -> Result<StreamChunk> {
                Ok(chunk.clone())
            }
        }

        #[derive(Clone)]
        struct MyTeam {
            leader: Leader,
        }

        impl Team for MyTeam {
            type Leader = Leader;
            fn leader(&self) -> &Leader {
                &self.leader
            }
            fn workers(&self) -> Vec<Tool> {
                vec![tool("analyst", "market analysis")]
            }
            async fn call(&self, _name: &str, input: String) -> Result<String> {
                Ok(format!("echo: {input}"))
            }
        }

        let team = MyTeam { leader: Leader };
        let tools = Agent::tools(&team);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "leader_tool");
        assert_eq!(tools[1].name, "analyst");
    }
}
