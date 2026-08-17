//! Programs — running an agent through an ordered sequence of bounded turns.
//!
//! Orchestration, not a chat: steps share one unpersisted history, so a later
//! step sees what earlier ones produced. For a fixed plan of deliverables — one
//! per step — rather than one open-ended turn.

use super::Runtime;
use crate::{AgentEvent, AgentResponse};
use crate::{Config, Env};
use async_stream::stream;
use futures_core::Stream;
use futures_util::StreamExt;
use storage::HistoryEntry;

/// One step of a [`Program`] — a single bounded turn's prompt.
pub struct ProgramStep {
    pub prompt: String,
}

impl From<String> for ProgramStep {
    fn from(prompt: String) -> Self {
        Self { prompt }
    }
}

/// An ordered plan run by [`Runtime::run_program`].
pub struct Program {
    pub steps: Vec<ProgramStep>,
}

impl From<Vec<String>> for Program {
    fn from(prompts: Vec<String>) -> Self {
        Self {
            steps: prompts.into_iter().map(ProgramStep::from).collect(),
        }
    }
}

impl<C: Config> Runtime<C> {
    /// A flat `AgentEvent` stream across every step, each ending in one
    /// `AgentEvent::Done` — so an observer counts `Done`s for progress. Like
    /// `ephemeral_stream`, dispatch runs with no conversation id.
    pub fn run_program<'a>(
        &'a self,
        agent_name: &'a str,
        program: Program,
        correlation_id: u64,
    ) -> impl Stream<Item = AgentEvent> + 'a {
        stream! {
            let Some(agent) = self.resolve_agent(agent_name) else {
                yield AgentEvent::Done(AgentResponse::error(
                    format!("agent '{agent_name}' not registered"),
                ));
                return;
            };

            let mut history = Vec::new();
            for step in program.steps {
                history.push(HistoryEntry::user(&step.prompt));
                let mut event_stream =
                    std::pin::pin!(agent.run_stream(&mut history, None, None, None));
                while let Some(event) = event_stream.next().await {
                    self.env
                        .on_agent_event(agent_name, correlation_id, true, &event);
                    yield event;
                }
            }
        }
    }
}
