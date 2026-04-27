//! Daemon-level conversation operations: send/stream (need os_hook cwd),
//! kill (needs os_hook cwd cleanup), and reply_to_ask (needs ask_hook).
//! Pure-runtime ops live on `Runtime<C>` directly.

use crate::daemon::Daemon;
use anyhow::Result;
use crabllm_core::Provider;
use futures_util::{StreamExt, pin_mut};
use std::sync::Arc;
use wcore::AgentEvent;
use wcore::protocol::message::*;

impl<P: Provider + 'static> Daemon<P> {
    pub(crate) async fn send(&self, req: SendMsg) -> Result<SendResponse> {
        let rt: Arc<_> = self.runtime.read().await.clone();
        let sender = req.sender.as_deref().unwrap_or("");
        let created_by = if sender.is_empty() { "user" } else { sender };
        let cwd = req.cwd.map(std::path::PathBuf::from);
        let conversation_id = rt
            .get_or_create_conversation(&req.agent, created_by)
            .await?;
        if let Some(ref cwd) = cwd {
            self.os_hook
                .conversation_cwds()
                .lock()
                .await
                .insert(conversation_id, cwd.clone());
        }
        let tool_choice = req
            .tool_choice
            .map(|s| wcore::model::ToolChoice::from(s.as_str()));
        let response = rt
            .send_to(conversation_id, &req.content, sender, tool_choice)
            .await?;
        Ok(SendResponse {
            agent: req.agent,
            content: response.final_response.unwrap_or_default(),
            model: response.model,
            usage: Some(sum_usage(&response.steps)),
        })
    }

    pub(crate) fn stream<'a>(
        &'a self,
        req: StreamMsg,
    ) -> impl futures_core::Stream<Item = Result<StreamEvent>> + Send + 'a {
        let runtime = self.runtime.clone();
        let conversation_cwds = self.os_hook.conversation_cwds().clone();
        let agent = req.agent;
        let content = req.content;
        let sender = req.sender.unwrap_or_default();
        let cwd = req.cwd.map(std::path::PathBuf::from);
        let guest = req.guest.unwrap_or_default();
        let tool_choice = req
            .tool_choice
            .map(|s| wcore::model::ToolChoice::from(s.as_str()));
        async_stream::try_stream! {
            let rt: Arc<_> = runtime.read().await.clone();
            let created_by = if sender.is_empty() { "user".into() } else { sender.clone() };
            let conversation_id = rt.get_or_create_conversation(&agent, created_by.as_str()).await?;
            if let Some(ref cwd) = cwd {
                conversation_cwds.lock().await.insert(conversation_id, cwd.clone());
            }

            let responding_agent = if guest.is_empty() { agent.clone() } else { guest.clone() };
            yield StreamEvent { event: Some(stream_event::Event::Start(StreamStart { agent: responding_agent.clone() })) };

            let stream: std::pin::Pin<Box<dyn futures_core::Stream<Item = wcore::AgentEvent> + Send + '_>> = if guest.is_empty() {
                Box::pin(rt.stream_to(conversation_id, &content, &sender, tool_choice))
            } else {
                Box::pin(rt.guest_stream_to(conversation_id, &content, &sender, &guest))
            };
            pin_mut!(stream);
            while let Some(event) = stream.next().await {
                match event {
                    AgentEvent::TextStart => {
                        yield StreamEvent { event: Some(stream_event::Event::TextStart(TextStartEvent { agent: responding_agent.clone() })) };
                    }
                    AgentEvent::TextDelta(text) => {
                        yield StreamEvent { event: Some(stream_event::Event::Chunk(StreamChunk { content: text })) };
                    }
                    AgentEvent::TextEnd => {
                        yield StreamEvent { event: Some(stream_event::Event::TextEnd(TextEndEvent { agent: responding_agent.clone() })) };
                    }
                    AgentEvent::ThinkingStart => {
                        yield StreamEvent { event: Some(stream_event::Event::ThinkingStart(ThinkingStartEvent { agent: responding_agent.clone() })) };
                    }
                    AgentEvent::ThinkingDelta(text) => {
                        yield StreamEvent { event: Some(stream_event::Event::Thinking(StreamThinking { content: text })) };
                    }
                    AgentEvent::ThinkingEnd => {
                        yield StreamEvent { event: Some(stream_event::Event::ThinkingEnd(ThinkingEndEvent { agent: responding_agent.clone() })) };
                    }
                    AgentEvent::ToolCallsBegin(calls) => {
                        yield StreamEvent { event: Some(stream_event::Event::ToolStart(ToolStartEvent {
                            calls: calls.into_iter().map(|c| ToolCallInfo {
                                name: c.function.name.to_string(),
                                arguments: String::new(),
                            }).collect(),
                        })) };
                    }
                    AgentEvent::ToolCallsStart(calls) => {
                        let ask_questions: Vec<AskQuestion> = calls
                            .iter()
                            .filter(|c| c.function.name == "ask_user")
                            .filter_map(|c| {
                                serde_json::from_str::<crate::hooks::ask_user::AskUser>(&c.function.arguments)
                                    .ok()
                            })
                            .flat_map(|a| a.questions)
                            .map(|q| AskQuestion {
                                question: q.question,
                                header: q.header,
                                options: q.options.into_iter().map(|o| AskOption {
                                    label: o.label,
                                    description: o.description,
                                }).collect(),
                                multi_select: q.multi_select,
                            })
                            .collect();

                        yield StreamEvent { event: Some(stream_event::Event::ToolStart(ToolStartEvent {
                            calls: calls.into_iter().map(|c| ToolCallInfo {
                                name: c.function.name.to_string(),
                                arguments: c.function.arguments,
                            }).collect(),
                        })) };

                        if !ask_questions.is_empty() {
                            yield StreamEvent { event: Some(stream_event::Event::AskUser(AskUserEvent { questions: ask_questions })) };
                        }
                    }
                    AgentEvent::ToolResult { call_id, output, duration_ms } => {
                        let is_error = output.is_err();
                        let output = match output { Ok(s) | Err(s) => s };
                        yield StreamEvent { event: Some(stream_event::Event::ToolResult(ToolResultEvent { call_id: call_id.to_string(), output, duration_ms, is_error })) };
                    }
                    AgentEvent::ToolCallsComplete => {
                        yield StreamEvent { event: Some(stream_event::Event::ToolsComplete(ToolsCompleteEvent {})) };
                    }
                    AgentEvent::ContextUsage { ref usage } => {
                        yield StreamEvent { event: Some(stream_event::Event::ContextUsage(ContextUsageEvent { usage: Some(usage_to_proto(usage)) })) };
                    }
                    AgentEvent::UserSteered { ref content } => {
                        yield StreamEvent { event: Some(stream_event::Event::UserSteered(UserSteeredEvent { content: content.clone() })) };
                    }
                    AgentEvent::Done(resp) => {
                        let error = if let wcore::AgentStopReason::Error(ref e) = resp.stop_reason {
                            e.clone()
                        } else {
                            String::new()
                        };
                        yield StreamEvent { event: Some(stream_event::Event::End(StreamEnd {
                            agent: responding_agent.clone(),
                            error,
                            model: resp.model,
                            usage: Some(sum_usage(&resp.steps)),
                        })) };
                        return;
                    }
                }
            }
            yield StreamEvent { event: Some(stream_event::Event::End(StreamEnd {
                agent: responding_agent.clone(),
                error: String::new(),
                model: String::new(),
                usage: None,
            })) };
        }
    }

    pub(crate) async fn kill_conversation(&self, agent: &str, sender: &str) -> Result<bool> {
        let rt = self.runtime.read().await.clone();
        let Some(conversation_id) = rt.conversation_id(agent, sender).await else {
            return Ok(false);
        };
        self.os_hook
            .conversation_cwds()
            .lock()
            .await
            .remove(&conversation_id);
        Ok(rt.close(conversation_id).await)
    }

    pub(crate) async fn reply_to_ask(
        &self,
        agent: &str,
        sender: &str,
        content: String,
    ) -> Result<()> {
        let rt = self.runtime.read().await.clone();
        let conversation_id = rt.require_conversation_id(agent, sender).await?;
        if let Some(tx) = self
            .ask_hook
            .pending_asks()
            .lock()
            .await
            .remove(&conversation_id)
        {
            let _ = tx.send(content);
            return Ok(());
        }
        // Retry once after a short delay — the ask_user handler may not have
        // inserted the oneshot yet if the reply races the tool call.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Some(tx) = self
            .ask_hook
            .pending_asks()
            .lock()
            .await
            .remove(&conversation_id)
        {
            let _ = tx.send(content);
            return Ok(());
        }
        anyhow::bail!("no pending ask_user for agent='{agent}' sender='{sender}'")
    }
}

pub(super) fn sum_usage(steps: &[wcore::AgentStep]) -> TokenUsage {
    let mut prompt = 0u32;
    let mut completion = 0u32;
    let mut total = 0u32;
    let mut cache_hit = 0u32;
    let mut cache_miss = 0u32;
    let mut reasoning = 0u32;
    let mut has_cache_hit = false;
    let mut has_cache_miss = false;
    let mut has_reasoning = false;

    for step in steps {
        let u = &step.usage;
        prompt += u.prompt_tokens;
        completion += u.completion_tokens;
        total += u.total_tokens;
        if let Some(v) = u.prompt_cache_hit_tokens {
            cache_hit += v;
            has_cache_hit = true;
        }
        if let Some(v) = u.prompt_cache_miss_tokens {
            cache_miss += v;
            has_cache_miss = true;
        }
        if let Some(ref d) = u.completion_tokens_details
            && let Some(v) = d.reasoning_tokens
        {
            reasoning += v;
            has_reasoning = true;
        }
    }

    TokenUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        cache_hit_tokens: has_cache_hit.then_some(cache_hit),
        cache_miss_tokens: has_cache_miss.then_some(cache_miss),
        reasoning_tokens: has_reasoning.then_some(reasoning),
    }
}

fn usage_to_proto(u: &crabllm_core::Usage) -> TokenUsage {
    TokenUsage {
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
        cache_hit_tokens: u.prompt_cache_hit_tokens,
        cache_miss_tokens: u.prompt_cache_miss_tokens,
        reasoning_tokens: u
            .completion_tokens_details
            .as_ref()
            .and_then(|d| d.reasoning_tokens),
    }
}
