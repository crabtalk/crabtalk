//! Wire protocol message types — re-exported from the generated protobuf
//! types, with the conversions into and out of them.

pub use crate::protocol::proto::*;
use crate::{AgentEvent, AgentResponse};

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
                let usage = Some(resp.usage());
                Event::End(StreamEnd {
                    agent: responding_agent.to_string(),
                    error,
                    model: resp.model,
                    usage,
                })
            }
        };
        StreamEvent { event: Some(event) }
    }
}

impl AgentResponse {
    /// Total usage across every step of a run.
    pub fn usage(&self) -> TokenUsage {
        let mut prompt = 0u32;
        let mut completion = 0u32;
        let mut total = 0u32;
        let mut cache_hit = 0u32;
        let mut cache_miss = 0u32;
        let mut reasoning = 0u32;

        for step in &self.steps {
            let u = &step.usage;
            prompt += u.prompt_tokens();
            completion += u.completion_tokens();
            total += u.total_tokens();
            cache_hit += u.cache_read_tokens;
            cache_miss += u.cache_write_tokens;
            reasoning += u.reasoning_tokens;
        }

        TokenUsage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: total,
            cache_hit_tokens: (cache_hit > 0).then_some(cache_hit),
            cache_miss_tokens: (cache_miss > 0).then_some(cache_miss),
            reasoning_tokens: (reasoning > 0).then_some(reasoning),
        }
    }
}
