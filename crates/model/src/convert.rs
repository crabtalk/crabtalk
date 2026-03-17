//! Type conversion between wcore model types and crabtalk-core wire types.

use compact_str::CompactString;
use crabtalk_core::{
    ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse, ChunkChoice,
    Delta as CtDelta, FinishReason as CtFinishReason, FunctionCall as CtFunctionCall, FunctionDef,
    Message as CtMessage, Role as CtRole, Tool as CtTool, ToolCall as CtToolCall, ToolType,
    Usage as CtUsage,
};
use wcore::model::{
    Choice, CompletionMeta, CompletionTokensDetails, Delta, FinishReason, FunctionCall, Message,
    Request, Response, Role, StreamChunk, Tool, ToolCall, ToolChoice, Usage,
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
        tool_choice: req.tool_choice.as_ref().map(to_ct_tool_choice),
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
    let role = convert_role(&msg.role);

    let content = if msg.content.is_empty() {
        None
    } else {
        Some(serde_json::Value::String(msg.content.clone()))
    };

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
        role,
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

fn to_ct_tool_choice(tc: &ToolChoice) -> serde_json::Value {
    match tc {
        ToolChoice::None => serde_json::Value::String("none".to_string()),
        ToolChoice::Auto => serde_json::Value::String("auto".to_string()),
        ToolChoice::Required => serde_json::Value::String("required".to_string()),
        ToolChoice::Function(name) => serde_json::json!({
            "type": "function",
            "function": { "name": name.as_str() }
        }),
    }
}

/// Convert a crabtalk ChatCompletionResponse into a wcore Response.
pub fn from_ct_response(resp: ChatCompletionResponse) -> Response {
    Response {
        meta: CompletionMeta {
            id: CompactString::from(&resp.id),
            object: CompactString::from(&resp.object),
            created: resp.created,
            model: CompactString::from(&resp.model),
            system_fingerprint: resp.system_fingerprint.map(|s| CompactString::from(&*s)),
        },
        choices: resp
            .choices
            .into_iter()
            .map(|c| Choice {
                index: c.index,
                delta: from_ct_message_delta(&c.message),
                finish_reason: c.finish_reason.map(convert_finish_reason),
                logprobs: None,
            })
            .collect(),
        usage: resp.usage.map(from_ct_usage).unwrap_or_default(),
    }
}

/// Convert a crabtalk ChatCompletionChunk into a wcore StreamChunk.
pub fn from_ct_chunk(chunk: ChatCompletionChunk) -> StreamChunk {
    StreamChunk {
        meta: CompletionMeta {
            id: CompactString::from(&chunk.id),
            object: CompactString::from(&chunk.object),
            created: chunk.created,
            model: CompactString::from(&chunk.model),
            system_fingerprint: chunk.system_fingerprint.map(|s| CompactString::from(&*s)),
        },
        choices: chunk
            .choices
            .into_iter()
            .map(from_ct_chunk_choice)
            .collect(),
        usage: chunk.usage.map(from_ct_usage),
    }
}

fn from_ct_chunk_choice(c: ChunkChoice) -> Choice {
    Choice {
        index: c.index,
        delta: from_ct_delta(&c.delta),
        finish_reason: c.finish_reason.map(convert_finish_reason),
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
                    id: CompactString::from(&tc.id),
                    index: tc.index.unwrap_or(0),
                    call_type: CompactString::from("function"),
                    function: FunctionCall {
                        name: CompactString::from(&tc.function.name),
                        arguments: tc.function.arguments.clone(),
                    },
                })
                .collect()
        }),
    }
}

fn from_ct_delta(d: &CtDelta) -> Delta {
    Delta {
        role: d.role.as_ref().map(convert_ct_role),
        content: d.content.clone(),
        reasoning_content: d.reasoning_content.clone(),
        tool_calls: d.tool_calls.as_ref().map(|tcs| {
            tcs.iter()
                .map(|tc| ToolCall {
                    id: tc
                        .id
                        .as_ref()
                        .map(|s| CompactString::from(&**s))
                        .unwrap_or_default(),
                    index: tc.index,
                    call_type: tc
                        .kind
                        .map(|k| {
                            CompactString::from(match k {
                                ToolType::Function => "function",
                            })
                        })
                        .unwrap_or_default(),
                    function: tc
                        .function
                        .as_ref()
                        .map(|f| FunctionCall {
                            name: f
                                .name
                                .as_ref()
                                .map(|s| CompactString::from(&**s))
                                .unwrap_or_default(),
                            arguments: f.arguments.clone().unwrap_or_default(),
                        })
                        .unwrap_or_default(),
                })
                .collect()
        }),
    }
}

fn from_ct_usage(u: CtUsage) -> Usage {
    Usage {
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
        prompt_cache_hit_tokens: u.prompt_cache_hit_tokens,
        prompt_cache_miss_tokens: u.prompt_cache_miss_tokens,
        completion_tokens_details: u
            .completion_tokens_details
            .map(|d| CompletionTokensDetails {
                reasoning_tokens: d.reasoning_tokens,
            }),
    }
}

fn convert_role(role: &Role) -> CtRole {
    match role {
        Role::User => CtRole::User,
        Role::Assistant => CtRole::Assistant,
        Role::System => CtRole::System,
        Role::Tool => CtRole::Tool,
    }
}

fn convert_ct_role(role: &CtRole) -> Role {
    match role {
        CtRole::User => Role::User,
        CtRole::Assistant => Role::Assistant,
        CtRole::System => Role::System,
        CtRole::Tool => Role::Tool,
        CtRole::Developer | CtRole::Custom(_) => Role::User,
    }
}

fn convert_finish_reason(reason: CtFinishReason) -> FinishReason {
    match reason {
        CtFinishReason::Stop => FinishReason::Stop,
        CtFinishReason::Length => FinishReason::Length,
        CtFinishReason::ToolCalls => FinishReason::ToolCalls,
        CtFinishReason::ContentFilter => FinishReason::ContentFilter,
        CtFinishReason::Custom(_) => FinishReason::Stop,
    }
}
