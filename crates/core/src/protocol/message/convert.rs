//! Conversions between protocol message types.

use crate::AgentEvent;
use crate::agent::AgentConfig;
use crate::protocol::proto::{
    AgentEventMsg, AgentInfo, ClientMessage, ContextUsageEvent, ConversationHistory, McpEventMsg,
    SendMsg, SendResponse, ServerMessage, StreamChunk, StreamEnd, StreamEvent, StreamMsg,
    StreamThinking, TextEndEvent, TextStartEvent, ThinkingEndEvent, ThinkingStartEvent, TokenUsage,
    ToolCallInfo, ToolDef, ToolResultEvent, ToolStartEvent, ToolsCompleteEvent, UserSteeredEvent,
    client_message, server_message, stream_event,
};

impl From<&AgentConfig> for AgentInfo {
    fn from(config: &AgentConfig) -> Self {
        Self {
            name: config.name.clone(),
            description: config.description.clone(),
            config: serde_json::to_string(config).unwrap_or_default(),
            model: config.model.clone(),
            max_iterations: config.max_iterations as u32,
            thinking: config.thinking,
            skills: config.skills.clone(),
            mcps: config.mcps.iter().map(|m| m.name.clone()).collect(),
            compact_tool_max_len: config.compact_tool_max_len as u32,
        }
    }
}

// ── ClientMessage constructors ───────────────────────────────────

impl From<SendMsg> for ClientMessage {
    fn from(msg: SendMsg) -> Self {
        Self {
            msg: Some(client_message::Msg::Send(msg)),
        }
    }
}

impl From<StreamMsg> for ClientMessage {
    fn from(msg: StreamMsg) -> Self {
        Self {
            msg: Some(client_message::Msg::Stream(msg)),
        }
    }
}

// ── ServerMessage constructors ───────────────────────────────────

impl From<SendResponse> for ServerMessage {
    fn from(r: SendResponse) -> Self {
        Self {
            msg: Some(server_message::Msg::Response(r)),
        }
    }
}

impl From<StreamEvent> for ServerMessage {
    fn from(e: StreamEvent) -> Self {
        Self {
            msg: Some(server_message::Msg::Stream(e)),
        }
    }
}

impl From<AgentEventMsg> for ServerMessage {
    fn from(e: AgentEventMsg) -> Self {
        Self {
            msg: Some(server_message::Msg::AgentEvent(e)),
        }
    }
}

impl From<McpEventMsg> for ServerMessage {
    fn from(e: McpEventMsg) -> Self {
        Self {
            msg: Some(server_message::Msg::McpEvent(e)),
        }
    }
}

impl From<ConversationHistory> for ServerMessage {
    fn from(h: ConversationHistory) -> Self {
        Self {
            msg: Some(server_message::Msg::ConversationHistory(h)),
        }
    }
}

// ── TryFrom<ServerMessage> ───────────────────────────────────────

fn error_or_unexpected(msg: ServerMessage) -> anyhow::Error {
    match msg.msg {
        Some(server_message::Msg::Error(e)) => {
            anyhow::anyhow!("server error ({}): {}", e.code, e.message)
        }
        other => anyhow::anyhow!("unexpected response: {other:?}"),
    }
}

impl TryFrom<ServerMessage> for SendResponse {
    type Error = anyhow::Error;
    fn try_from(msg: ServerMessage) -> anyhow::Result<Self> {
        match msg.msg {
            Some(server_message::Msg::Response(r)) => Ok(r),
            _ => Err(error_or_unexpected(msg)),
        }
    }
}

impl TryFrom<ServerMessage> for stream_event::Event {
    type Error = anyhow::Error;
    fn try_from(msg: ServerMessage) -> anyhow::Result<Self> {
        match msg.msg {
            Some(server_message::Msg::Stream(e)) => {
                e.event.ok_or_else(|| anyhow::anyhow!("empty stream event"))
            }
            _ => Err(error_or_unexpected(msg)),
        }
    }
}

impl From<&crate::model::Usage> for TokenUsage {
    fn from(u: &crate::model::Usage) -> Self {
        Self {
            prompt_tokens: u.prompt_tokens(),
            completion_tokens: u.completion_tokens(),
            total_tokens: u.total_tokens(),
            cache_hit_tokens: (u.cache_read_tokens > 0).then_some(u.cache_read_tokens),
            cache_miss_tokens: (u.cache_write_tokens > 0).then_some(u.cache_write_tokens),
            reasoning_tokens: (u.reasoning_tokens > 0).then_some(u.reasoning_tokens),
        }
    }
}

/// A client tool as declared on the wire. The schema travels as a JSON
/// string, so an unparsable one degrades to "no parameters" rather than
/// failing the whole stream.
impl From<ToolDef> for crate::model::Tool {
    fn from(def: ToolDef) -> Self {
        Self {
            kind: crate::model::ToolType::Function,
            function: crate::model::FunctionDef {
                name: def.name,
                description: (!def.description.is_empty()).then_some(def.description),
                parameters: (!def.parameters_schema.is_empty())
                    .then(|| serde_json::from_str(&def.parameters_schema).ok())
                    .flatten(),
            },
            strict: None,
        }
    }
}

impl From<crate::model::Tool> for ToolDef {
    fn from(tool: crate::model::Tool) -> Self {
        Self {
            name: tool.function.name,
            description: tool.function.description.unwrap_or_default(),
            parameters_schema: tool
                .function
                .parameters
                .map(|p| p.to_string())
                .unwrap_or_default(),
        }
    }
}

impl AgentEvent {
    /// Map one agent loop event to its wire `StreamEvent`. Shared by the
    /// persisted and ephemeral stream paths; client-tool forwarding (which
    /// only the persisted path can route) is handled by the caller.
    pub fn to_stream(self, responding_agent: &str) -> StreamEvent {
        let event = self;
        use stream_event::Event;
        let event = match event {
            AgentEvent::TextStart => Event::TextStart(TextStartEvent {
                agent: responding_agent.to_string(),
            }),
            AgentEvent::TextDelta(text) => Event::Chunk(StreamChunk { content: text }),
            AgentEvent::TextEnd => Event::TextEnd(TextEndEvent {
                agent: responding_agent.to_string(),
            }),
            AgentEvent::ThinkingStart => Event::ThinkingStart(ThinkingStartEvent {
                agent: responding_agent.to_string(),
            }),
            AgentEvent::ThinkingDelta(text) => Event::Thinking(StreamThinking { content: text }),
            AgentEvent::ThinkingEnd => Event::ThinkingEnd(ThinkingEndEvent {
                agent: responding_agent.to_string(),
            }),
            AgentEvent::ToolCallsBegin(calls) => Event::ToolStart(ToolStartEvent {
                calls: calls
                    .into_iter()
                    .map(|c| ToolCallInfo {
                        name: c.function.name.to_string(),
                        arguments: String::new(),
                    })
                    .collect(),
            }),
            AgentEvent::ToolCallsStart(calls) => Event::ToolStart(ToolStartEvent {
                calls: calls
                    .into_iter()
                    .map(|c| ToolCallInfo {
                        name: c.function.name.to_string(),
                        arguments: c.function.arguments,
                    })
                    .collect(),
            }),
            AgentEvent::ToolResult {
                call_id,
                output,
                duration_ms,
            } => {
                let is_error = output.is_err();
                let output = match output {
                    Ok(s) | Err(s) => s,
                };
                Event::ToolResult(ToolResultEvent {
                    call_id: call_id.to_string(),
                    output,
                    duration_ms,
                    is_error,
                })
            }
            AgentEvent::ToolCallsComplete => Event::ToolsComplete(ToolsCompleteEvent {}),
            AgentEvent::ContextUsage { usage } => Event::ContextUsage(ContextUsageEvent {
                usage: Some((&usage).into()),
            }),
            AgentEvent::UserSteered { content } => Event::UserSteered(UserSteeredEvent { content }),
            AgentEvent::Done(resp) => {
                let error = if let crate::AgentStopReason::Error(ref e) = resp.stop_reason {
                    e.clone()
                } else {
                    String::new()
                };
                Event::End(StreamEnd {
                    agent: responding_agent.to_string(),
                    error,
                    model: resp.model,
                    usage: Some(TokenUsage::sum(&resp.steps)),
                })
            }
        };
        StreamEvent { event: Some(event) }
    }
}

impl TokenUsage {
    /// Total usage across every step of a run.
    pub fn sum(steps: &[crate::AgentStep]) -> Self {
        let mut prompt = 0u32;
        let mut completion = 0u32;
        let mut total = 0u32;
        let mut cache_hit = 0u32;
        let mut cache_miss = 0u32;
        let mut reasoning = 0u32;

        for step in steps {
            let u = &step.usage;
            prompt += u.prompt_tokens();
            completion += u.completion_tokens();
            total += u.total_tokens();
            cache_hit += u.cache_read_tokens;
            cache_miss += u.cache_write_tokens;
            reasoning += u.reasoning_tokens;
        }

        Self {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: total,
            cache_hit_tokens: (cache_hit > 0).then_some(cache_hit),
            cache_miss_tokens: (cache_miss > 0).then_some(cache_miss),
            reasoning_tokens: (reasoning > 0).then_some(reasoning),
        }
    }
}
