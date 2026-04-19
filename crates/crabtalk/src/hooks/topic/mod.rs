//! Topic hook — `search_topics` and `switch_topic`. Topics partition
//! `(agent, sender)` into parallel conversation threads; see RFC #171.
//!
//! The hook itself owns no state — it holds a late-bind runtime handle
//! (to drive `switch_topic`) and a shared memory handle (to BM25
//! over `EntryKind::Topic` entries).

use crabllm_core::Provider;
use runtime::{Hook, SharedMemory};
use std::sync::{Arc, OnceLock};
use wcore::{ToolDispatch, ToolFuture, agent::AsTool, model::Tool};

use crate::daemon::SharedRuntime;

mod search;
mod switch;

use search::SearchTopics;
use switch::SwitchTopic;

/// Behavioural guidance — when/how to use the topic tools. Tool
/// *signatures* come from each struct's `///` doc comment via schemars.
const TOPIC_PROMPT: &str = include_str!("../../../prompts/topic.md");

pub struct TopicHook<P: Provider + 'static> {
    pub(super) runtime: Arc<OnceLock<SharedRuntime<P>>>,
    pub(super) memory: SharedMemory,
}

impl<P: Provider + 'static> TopicHook<P> {
    pub fn new(runtime: Arc<OnceLock<SharedRuntime<P>>>, memory: SharedMemory) -> Self {
        Self { runtime, memory }
    }
}

impl<P: Provider + 'static> Hook for TopicHook<P> {
    fn schema(&self) -> Vec<Tool> {
        vec![SearchTopics::as_tool(), SwitchTopic::as_tool()]
    }

    fn system_prompt(&self) -> Option<String> {
        Some(format!("\n\n{TOPIC_PROMPT}"))
    }

    fn dispatch<'a>(&'a self, name: &'a str, call: ToolDispatch) -> Option<ToolFuture<'a>> {
        match name {
            "search_topics" => Some(Box::pin(self.handle_search_topics(call))),
            "switch_topic" => Some(Box::pin(self.handle_switch_topic(call))),
            _ => None,
        }
    }
}
