//! Tool schema and dispatch for the built-in `ask_user` tool.

use crate::hook::DaemonHook;
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::oneshot;
use wcore::{
    agent::{AsTool, ToolDescription},
    model::Tool,
};

/// A single option the user can choose from.
#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct QuestionOption {
    /// Concise option label (1-5 words).
    pub label: String,
    /// Explanation of the choice.
    pub description: String,
}

/// A structured question with predefined options.
#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct Question {
    /// Full question text.
    pub question: String,
    /// Short UI title for the question (max 12 chars, e.g. "Database").
    pub header: String,
    /// Predefined choices for the user.
    pub options: Vec<QuestionOption>,
    /// Allow multiple selections.
    #[serde(default)]
    pub multi_select: bool,
}

/// Ask the user one or more structured questions and wait for their reply.
#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct AskUser {
    /// The questions to ask the user.
    pub questions: Vec<Question>,
}

impl ToolDescription for AskUser {
    const DESCRIPTION: &'static str = r#"Ask the user one or more structured questions with predefined options. Each question needs a short UI header, the full question text, and options with labels and descriptions. The user picks from the options or types a free-text "Other" answer. Returns JSON mapping question text to selected label. For multi_select, the answer is a comma-joined string like "Option A, Option B"."#;
}

pub(crate) fn tools() -> Vec<Tool> {
    vec![AskUser::as_tool()]
}

/// Timeout for waiting on user reply (5 minutes).
const ASK_USER_TIMEOUT: Duration = Duration::from_secs(300);

impl DaemonHook {
    pub(crate) async fn dispatch_ask_user(&self, args: &str, session_id: Option<u64>) -> String {
        let input: AskUser = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };

        let session_id = match session_id {
            Some(id) => id,
            None => return "ask_user is only available in streaming mode".to_owned(),
        };

        let (tx, rx) = oneshot::channel();
        self.pending_asks.lock().await.insert(session_id, tx);

        match tokio::time::timeout(ASK_USER_TIMEOUT, rx).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => {
                self.pending_asks.lock().await.remove(&session_id);
                "ask_user cancelled: reply channel closed".to_owned()
            }
            Err(_) => {
                self.pending_asks.lock().await.remove(&session_id);
                let headers: Vec<&str> =
                    input.questions.iter().map(|q| q.header.as_str()).collect();
                format!(
                    "ask_user timed out after {}s: no reply received for: {}",
                    ASK_USER_TIMEOUT.as_secs(),
                    headers.join("; "),
                )
            }
        }
    }
}
