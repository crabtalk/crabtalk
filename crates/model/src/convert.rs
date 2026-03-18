//! Type conversion between wcore model types and crabtalk-core wire types.
//!
//! Role and FinishReason are shared (re-exported from crabtalk-core in wcore).
//! Structural differences remain: String vs Option<Value> content, flat vs
//! envelope Tool.

use crabtalk_core::{
    ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse, ChunkChoice,
    Delta as CtDelta, FunctionCall as CtFunctionCall, FunctionDef, Message as CtMessage,
    Tool as CtTool, ToolCall as CtToolCall, ToolType,
};
use wcore::model::{
    Choice, CompletionMeta, Delta, FunctionCall, Message, Request, Response, StreamChunk, Tool,
    ToolCall,
};

/// Convert a wcore Request into a crabtalk ChatCompletionRequest.
pub fn to_ct_request(req: &Request) -> ChatCompletionRequest {
    ChatCompletionRequest {
        model: req.model.to_string(),
        messages: req.messages.iter().map(to_ct_message).collect(),
        temperature: None,
        top_p: None,
        max_tokens: None,
        stream: None, // caller sets this
        stop: None,
        tools: req
            .tools
            .as_ref()
            .map(|ts| ts.iter().map(to_ct_tool).collect()),
        tool_choice: req.tool_choice.clone(),
        frequency_penalty: None,
        presence_penalty: None,
        seed: None,
        user: None,
        reasoning_effort: if req.think {
            Some("high".to_string())
        } else {
            None
        },
        extra: Default::default(),
    }
}

fn to_ct_message(msg: &Message) -> CtMessage {
    // Always include `content` — the OpenAI API requires the field on
    // every message. Assistant messages accept `null`; all other roles
    // require a string.
    let content = Some(if msg.content.is_empty() {
        if msg.role == crabtalk_core::Role::Assistant {
            serde_json::Value::Null
        } else {
            serde_json::Value::String(String::new())
        }
    } else {
        serde_json::Value::String(msg.content.clone())
    });

    let tool_calls = if msg.tool_calls.is_empty() {
        None
    } else {
        Some(msg.tool_calls.iter().map(to_ct_tool_call).collect())
    };

    let tool_call_id = if msg.tool_call_id.is_empty() {
        None
    } else {
        Some(msg.tool_call_id.to_string())
    };

    let reasoning_content = if msg.reasoning_content.is_empty() {
        None
    } else {
        Some(msg.reasoning_content.clone())
    };

    CtMessage {
        role: msg.role.clone(),
        content,
        tool_calls,
        tool_call_id,
        name: None,
        reasoning_content,
        extra: Default::default(),
    }
}

fn to_ct_tool(tool: &Tool) -> CtTool {
    CtTool {
        kind: ToolType::Function,
        function: FunctionDef {
            name: tool.name.to_string(),
            description: Some(tool.description.to_string()),
            parameters: Some(serde_json::to_value(&tool.parameters).unwrap_or_default()),
        },
        strict: if tool.strict { Some(true) } else { None },
    }
}

fn to_ct_tool_call(tc: &ToolCall) -> CtToolCall {
    CtToolCall {
        index: Some(tc.index),
        id: tc.id.to_string(),
        kind: ToolType::Function,
        function: CtFunctionCall {
            name: tc.function.name.to_string(),
            arguments: tc.function.arguments.clone(),
        },
    }
}

/// Convert a crabtalk ChatCompletionResponse into a wcore Response.
pub fn from_ct_response(resp: ChatCompletionResponse) -> Response {
    let meta = CompletionMeta {
        id: resp.id,
        object: resp.object,
        created: resp.created,
        model: resp.model,
        system_fingerprint: resp.system_fingerprint,
    };
    Response {
        meta,
        choices: resp
            .choices
            .into_iter()
            .map(|c| Choice {
                index: c.index,
                delta: from_ct_message_delta(&c.message),
                finish_reason: c.finish_reason,
                logprobs: None,
            })
            .collect(),
        usage: resp.usage.unwrap_or_default(),
    }
}

/// Convert a crabtalk ChatCompletionChunk into a wcore StreamChunk.
pub fn from_ct_chunk(chunk: ChatCompletionChunk) -> StreamChunk {
    let meta = CompletionMeta {
        id: chunk.id,
        object: chunk.object,
        created: chunk.created,
        model: chunk.model,
        system_fingerprint: chunk.system_fingerprint,
    };
    StreamChunk {
        meta,
        choices: chunk
            .choices
            .into_iter()
            .map(from_ct_chunk_choice)
            .collect(),
        usage: chunk.usage,
    }
}

fn from_ct_chunk_choice(c: ChunkChoice) -> Choice {
    Choice {
        index: c.index,
        delta: from_ct_delta(&c.delta),
        finish_reason: c.finish_reason,
        logprobs: None,
    }
}

fn from_ct_message_delta(msg: &CtMessage) -> Delta {
    let content = msg
        .content
        .as_ref()
        .and_then(|v| match v {
            serde_json::Value::String(s) => Some(s.clone()),
            _ => None,
        })
        .filter(|s| !s.is_empty());

    Delta {
        role: None,
        content,
        reasoning_content: msg.reasoning_content.clone(),
        tool_calls: msg.tool_calls.as_ref().map(|tcs| {
            tcs.iter()
                .map(|tc| ToolCall {
                    id: tc.id.clone(),
                    index: tc.index.unwrap_or(0),
                    call_type: "function".to_owned(),
                    function: FunctionCall {
                        name: tc.function.name.clone(),
                        arguments: tc.function.arguments.clone(),
                    },
                })
                .collect()
        }),
    }
}

fn from_ct_delta(d: &CtDelta) -> Delta {
    Delta {
        role: d.role.clone(),
        content: d.content.clone(),
        reasoning_content: d.reasoning_content.clone(),
        tool_calls: d.tool_calls.as_ref().map(|tcs| {
            tcs.iter()
                .map(|tc| ToolCall {
                    id: tc.id.as_ref().map(|s| s.to_string()).unwrap_or_default(),
                    index: tc.index,
                    call_type: tc
                        .kind
                        .map(|k| {
                            match k {
                                ToolType::Function => "function",
                            }
                            .to_owned()
                        })
                        .unwrap_or_default(),
                    function: tc
                        .function
                        .as_ref()
                        .map(|f| FunctionCall {
                            name: f.name.as_ref().map(|s| s.to_string()).unwrap_or_default(),
                            arguments: f.arguments.clone().unwrap_or_default(),
                        })
                        .unwrap_or_default(),
                })
                .collect()
        }),
    }
}
