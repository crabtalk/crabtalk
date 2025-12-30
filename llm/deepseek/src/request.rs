//! The request body for the DeepSeek API

use ccore::{Config, General, Message, Tool, ToolChoice};
use serde::Serialize;
use serde_json::{Value, json};

/// The request body for the DeepSeek API
#[derive(Debug, Clone, Serialize)]
pub struct Request {
    /// The messages to send to the API
    pub messages: Vec<Message>,

    /// The model we are using
    pub model: String,

    /// The frequency penalty to use for the response
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<Value>,

    /// Whether to return the log probabilities
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<Value>,

    /// The maximum number of tokens to generate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,

    /// The presence penalty to use for the response
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<Value>,

    /// The response format to use
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<Value>,

    /// Stop sequences
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Value>,

    /// Whether to stream the response
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,

    /// Stream options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<Value>,

    /// Whether to enable thinking
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<Value>,

    /// The temperature to use for the response
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Value>,

    /// Controls which (if any) tool is called by the model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,

    /// A list of tools the model may call
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Value>,

    /// An integer between 0 and 20 specifying the number of most likely tokens to
    /// return at each token position, each with an associated log probability.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<Value>,

    /// The top probability to use for the response
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<Value>,
}

impl Request {
    /// Construct the messages for the request
    pub fn messages(&self, messages: &[Message]) -> Self {
        Self {
            messages: messages.to_vec(),
            ..self.clone()
        }
    }

    /// Enable streaming for the request
    pub fn stream(mut self, usage: bool) -> Self {
        self.stream = Some(true);
        self.stream_options = if usage {
            Some(json!({ "include_usage": true }))
        } else {
            None
        };
        self
    }
}

impl From<General> for Request {
    fn from(config: General) -> Self {
        Self {
            messages: Vec::new(),
            model: config.model.clone(),
            frequency_penalty: None,
            logprobs: None,
            max_tokens: None,
            presence_penalty: None,
            response_format: None,
            stop: None,
            stream: None,
            stream_options: None,
            thinking: if config.think {
                Some(json!({
                    "type": "enabled"
                }))
            } else {
                None
            },
            temperature: None,
            tool_choice: None,
            tools: None,
            top_logprobs: None,
            top_p: None,
        }
    }
}

impl Config for Request {
    fn with_tools(self, tools: Vec<Tool>) -> Self {
        let tools = tools
            .into_iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": json!(tool),
                })
            })
            .collect::<Vec<_>>();
        Self {
            tools: Some(json!(tools)),
            ..self.clone()
        }
    }

    fn with_tool_choice(&self, tool_choice: ToolChoice) -> Self {
        Self {
            tool_choice: match tool_choice {
                ToolChoice::Function(name) => Some(json!({
                    "type": "function",
                    "function": json!({
                        "name": name
                    })
                })),
                _ => None,
            },
            ..self.clone()
        }
    }
}
