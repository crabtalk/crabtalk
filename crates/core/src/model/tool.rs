//! Tool abstractions for the unified LLM Interfaces

use compact_str::CompactString;
use schemars::Schema;
use serde::{Deserialize, Serialize};

/// A tool for the LLM
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Tool {
    /// The name of the tool
    pub name: CompactString,

    /// The description of the tool
    pub description: CompactString,

    /// The parameters of the tool
    pub parameters: Schema,

    /// Whether to strictly validate the parameters
    pub strict: bool,
}

/// A tool call made by the model
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ToolCall {
    /// The ID of the tool call
    #[serde(default, skip_serializing_if = "CompactString::is_empty")]
    pub id: CompactString,

    /// The index of the tool call (used in streaming)
    #[serde(default, skip_serializing)]
    pub index: u32,

    /// The type of tool (currently only "function")
    #[serde(default, rename = "type")]
    pub call_type: CompactString,

    /// The function to call
    pub function: FunctionCall,
}

impl ToolCall {
    /// Merge two tool calls into one
    pub fn merge(&mut self, call: &Self) {
        if !call.id.is_empty() {
            self.id = call.id.clone();
        }
        if !call.call_type.is_empty() {
            self.call_type = call.call_type.clone();
        }
        if !call.function.name.is_empty() {
            self.function.name = call.function.name.clone();
        }
        self.function.arguments.push_str(&call.function.arguments);
    }
}

/// A function call within a tool call
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FunctionCall {
    /// The name of the function to call
    #[serde(default, skip_serializing_if = "CompactString::is_empty")]
    pub name: CompactString,

    /// The arguments to pass to the function (JSON string)
    #[serde(skip_serializing_if = "String::is_empty")]
    pub arguments: String,
}

pub use crabtalk_core::ToolChoice;
