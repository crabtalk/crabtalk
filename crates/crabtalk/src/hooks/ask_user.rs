//! Ask-user tool — as a Hook implementation.

use runtime::{Hook, PendingAsks};
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::oneshot;
use wcore::{
    ToolDispatch, ToolFuture,
    agent::{AsTool, ToolDescription},
};

/// Timeout for waiting on user reply (5 minutes).
const ASK_USER_TIMEOUT: Duration = Duration::from_secs(300);

/// A single option the user can choose from.
#[derive(Deserialize, schemars::JsonSchema)]
pub struct QuestionOption {
    /// Concise option label (1-5 words).
    pub label: String,
    /// Explanation of the choice.
    pub description: String,
}

/// A structured question with predefined options.
#[derive(Deserialize, schemars::JsonSchema)]
pub struct Question {
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
pub struct AskUser {
    /// The questions to ask the user.
    pub questions: Vec<Question>,
}

impl ToolDescription for AskUser {
    const DESCRIPTION: &'static str = r#"Ask the user one or more structured questions with predefined options. Each question needs a short UI header, the full question text, and options with labels and descriptions. The user picks from the options or types a free-text "Other" answer. Returns JSON mapping question text to selected label. For multi_select, the answer is a comma-joined string like "Option A, Option B"."#;
}

/// Ask-user subsystem.
///
/// Owns the pending-asks map shared with the protocol layer for reply
/// routing.
pub struct AskUserHook {
    pending_asks: PendingAsks,
}

impl AskUserHook {
    pub fn new(pending_asks: PendingAsks) -> Self {
        Self { pending_asks }
    }

    /// Access the shared pending-asks map (for protocol reply routing).
    pub fn pending_asks(&self) -> &PendingAsks {
        &self.pending_asks
    }
}

impl Hook for AskUserHook {
    fn schema(&self) -> Vec<wcore::model::Tool> {
        vec![AskUser::as_tool()]
    }

    fn dispatch<'a>(&'a self, name: &'a str, call: ToolDispatch) -> Option<ToolFuture<'a>> {
        if name != "ask_user" {
            return None;
        }
        Some(Box::pin(async move {
            let input: AskUser =
                serde_json::from_str(&call.args).map_err(|e| format!("invalid arguments: {e}"))?;

            let conversation_id = call
                .conversation_id
                .ok_or("ask_user is only available in streaming mode")?;

            let (tx, rx) = oneshot::channel();
            self.pending_asks.lock().await.insert(conversation_id, tx);

            match tokio::time::timeout(ASK_USER_TIMEOUT, rx).await {
                Ok(Ok(reply)) => Ok(reply),
                Ok(Err(_)) => {
                    self.pending_asks.lock().await.remove(&conversation_id);
                    Err("ask_user cancelled: reply channel closed".to_owned())
                }
                Err(_) => {
                    self.pending_asks.lock().await.remove(&conversation_id);
                    let headers: Vec<&str> =
                        input.questions.iter().map(|q| q.header.as_str()).collect();
                    Err(format!(
                        "ask_user timed out after {}s: no reply received for: {}",
                        ASK_USER_TIMEOUT.as_secs(),
                        headers.join("; "),
                    ))
                }
            }
        }))
    }
}
