//! Tool schema for the built-in `ask_user` tool.
//!
//! Schema types live here. Dispatch logic is server-specific and lives in
//! the [`RuntimeBridge`](crate::bridge::RuntimeBridge) implementation.

use serde::Deserialize;
use wcore::{
    agent::{AsTool, ToolDescription},
    model::Tool,
};

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

pub fn tools() -> Vec<Tool> {
    vec![AskUser::as_tool()]
}
