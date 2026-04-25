//! Sessions hook — `search_sessions`. BM25 over conversation messages
//! with windowed excerpts. See RFC 0185.

use crate::daemon::SharedRuntime;
use crabllm_core::Provider;
use runtime::Hook;
use search::SearchSessions;
use std::sync::{Arc, OnceLock};
use wcore::{ToolDispatch, ToolFuture, agent::AsTool, model::Tool};

mod search;

const SESSIONS_PROMPT: &str = include_str!("../../../prompts/sessions.md");

pub struct SessionsHook<P: Provider + 'static> {
    pub(super) runtime: Arc<OnceLock<SharedRuntime<P>>>,
}

impl<P: Provider + 'static> SessionsHook<P> {
    pub fn new(runtime: Arc<OnceLock<SharedRuntime<P>>>) -> Self {
        Self { runtime }
    }
}

impl<P: Provider + 'static> Hook for SessionsHook<P> {
    fn schema(&self) -> Vec<Tool> {
        vec![SearchSessions::as_tool()]
    }

    fn system_prompt(&self) -> Option<String> {
        Some(format!("\n\n{SESSIONS_PROMPT}"))
    }

    fn dispatch<'a>(&'a self, name: &'a str, call: ToolDispatch) -> Option<ToolFuture<'a>> {
        match name {
            "search_sessions" => Some(Box::pin(self.handle_search_sessions(call))),
            _ => None,
        }
    }
}
