//! Ask-user tool handler factory.

use serde::Deserialize;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::{Mutex, oneshot};
use wcore::{
    ToolDispatch, ToolHandler,
    agent::{AsTool, ToolDescription},
    model::Tool,
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

/// Pending ask_user oneshots, keyed by conversation_id.
pub type PendingAsks = Arc<Mutex<HashMap<u64, oneshot::Sender<String>>>>;

pub fn handler(pending_asks: PendingAsks) -> (Tool, ToolHandler) {
    (
        AskUser::as_tool(),
        Arc::new(move |call: ToolDispatch| {
            let pending_asks = pending_asks.clone();
            Box::pin(async move {
                let input: AskUser = serde_json::from_str(&call.args)
                    .map_err(|e| format!("invalid arguments: {e}"))?;

                let conversation_id = call
                    .conversation_id
                    .ok_or("ask_user is only available in streaming mode")?;

                let (tx, rx) = oneshot::channel();
                pending_asks.lock().await.insert(conversation_id, tx);

                match tokio::time::timeout(ASK_USER_TIMEOUT, rx).await {
                    Ok(Ok(reply)) => Ok(reply),
                    Ok(Err(_)) => {
                        pending_asks.lock().await.remove(&conversation_id);
                        Err("ask_user cancelled: reply channel closed".to_owned())
                    }
                    Err(_) => {
                        pending_asks.lock().await.remove(&conversation_id);
                        let headers: Vec<&str> =
                            input.questions.iter().map(|q| q.header.as_str()).collect();
                        Err(format!(
                            "ask_user timed out after {}s: no reply received for: {}",
                            ASK_USER_TIMEOUT.as_secs(),
                            headers.join("; "),
                        ))
                    }
                }
            })
        }),
    )
}
