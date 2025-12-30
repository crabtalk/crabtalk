//! Tool abstractions for the unified LLM Interfaces

use schemars::Schema;
use serde::{Deserialize, Serialize};

/// A tool for the LLM
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Tool {
    /// The name of the tool
    pub name: String,

    /// The description of the tool
    pub description: String,

    /// The parameters of the tool
    pub parameters: Schema,

    /// Whether to strictly validate the parameters
    pub strict: bool,
}

/// A tool call made by the model
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ToolCall {
    /// The ID of the tool call
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,

    /// The index of the tool call (used in streaming)
    #[serde(default, skip_serializing)]
    pub index: u32,

    /// The type of tool (currently only "function")
    #[serde(default, rename = "type")]
    pub call_type: String,

    /// The function to call
    pub function: FunctionCall,
}

impl ToolCall {
    /// Merge two tool calls into one
    pub fn merge(&mut self, call: &Self) {
        if !call.id.is_empty() {
            self.id.clone_from(&call.id);
        }
        if !call.call_type.is_empty() {
            self.call_type.clone_from(&call.call_type);
        }
        if !call.function.name.is_empty() {
            self.function.name.clone_from(&call.function.name);
        }
        self.function.arguments.push_str(&call.function.arguments);
    }
}

/// A function call within a tool call
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FunctionCall {
    /// The name of the function to call
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,

    /// The arguments to pass to the function (JSON string)
    #[serde(skip_serializing_if = "String::is_empty")]
    pub arguments: String,
}

/// Controls which tool is called by the model
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub enum ToolChoice {
    /// Model will not call any tool
    #[serde(rename = "none")]
    None,

    /// Model can pick between generating a message or calling tools
    #[serde(rename = "auto")]
    #[default]
    Auto,

    /// Model must call one or more tools
    #[serde(rename = "required")]
    Required,

    /// Model must call the specified function
    Function(String),
}

impl From<&str> for ToolChoice {
    fn from(value: &str) -> Self {
        ToolChoice::Function(value.into())
    }
}
