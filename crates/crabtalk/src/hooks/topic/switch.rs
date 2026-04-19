//! `switch_topic` — enter an existing topic or create a new one. On
//! create, writes an `EntryKind::Topic` memory entry so the topic is
//! searchable. On resume, flips the active-topic pointer for the
//! current `(agent, sender)`.

use super::TopicHook;
use crabllm_core::Provider;
use schemars::JsonSchema;
use serde::Deserialize;
use wcore::ToolDispatch;

/// Switch the active topic for this conversation. Exact title match
/// resumes that topic; a new title creates a fresh one (requires
/// `description`). Untopicked chats are tmp and not persisted —
/// entering a topic is how the agent promotes work into long-term
/// memory.
#[derive(Deserialize, JsonSchema)]
pub struct SwitchTopic {
    /// Topic title. Free-form; the title is the key.
    pub title: String,
    /// One- to three-sentence description of the topic. Required when
    /// creating a new topic; ignored when resuming an existing one.
    #[serde(default)]
    pub description: Option<String>,
}

impl<P: Provider + 'static> TopicHook<P> {
    pub(super) async fn handle_switch_topic(&self, call: ToolDispatch) -> Result<String, String> {
        let input: SwitchTopic =
            serde_json::from_str(&call.args).map_err(|e| format!("invalid arguments: {e}"))?;
        let shared = self
            .runtime
            .get()
            .ok_or_else(|| "switch_topic: runtime not initialized".to_owned())?;
        let rt = shared.read().await.clone();
        let outcome = rt
            .switch_topic(
                &call.agent,
                &call.sender,
                &input.title,
                input.description.as_deref(),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(if outcome.resumed {
            format!("resumed topic: {}", input.title)
        } else {
            format!("created topic: {}", input.title)
        })
    }
}
