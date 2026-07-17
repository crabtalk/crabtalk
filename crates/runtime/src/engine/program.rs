//! Programs — running an agent through an ordered sequence of bounded turns.
//!
//! A [`Program`] is orchestration, not a chat: its steps share one unpersisted
//! in-memory history, so a later step sees what earlier ones produced (and can
//! build on or avoid repeating them). Reach for it when a task is a fixed plan
//! of deliverables — one per step — rather than one open-ended turn.

use super::Runtime;
use crate::{Config, Env};
use async_stream::stream;
use futures_core::Stream;
use futures_util::StreamExt;
use wcore::{AgentEvent, AgentResponse, model::HistoryEntry};

/// One step of a [`Program`] — a single bounded turn's prompt. A struct rather
/// than a bare string so a step can grow its own scope or forced output later
/// without churning call sites.
pub struct ProgramStep {
    pub prompt: String,
}

impl From<String> for ProgramStep {
    fn from(prompt: String) -> Self {
        Self { prompt }
    }
}

/// An ordered plan run by [`Runtime::run_program`]. Each step is a bounded turn;
/// all steps share one history.
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
    /// Run a [`Program`]: each step is a bounded turn, all sharing one in-memory
    /// history. Yields a flat `AgentEvent` stream across every step; each step
    /// ends in one `AgentEvent::Done`, so an observer counts `Done`s to track
    /// progress (`step k of steps.len()`). Like `ephemeral_stream`, dispatch runs
    /// with no conversation id, so a program only reaches self-contained
    /// daemon-side tools.
    pub fn run_program<'a>(
        &'a self,
        agent_name: &'a str,
        program: Program,
        correlation_id: u64,
    ) -> impl Stream<Item = AgentEvent> + 'a {
        stream! {
            let Some(agent) = self.resolve_agent(agent_name).await else {
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
