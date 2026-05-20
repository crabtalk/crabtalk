//! Execution — message sending and streaming through agents.

use super::Runtime;
use crate::{Config, Conversation, Env, Hook};
use anyhow::Result;
use async_stream::stream;
use crabllm_core::{
    AnthropicContent, AnthropicMessage, AnthropicRequest, AnthropicSystem, DEFAULT_MAX_TOKENS,
    ThinkingConfig, ToolChoice,
};
use futures_core::Stream;
use futures_util::StreamExt;
use tokio::sync::{mpsc, watch};
use wcore::{AgentEvent, AgentResponse, AgentStopReason, model::HistoryEntry};

impl<C: Config> Runtime<C> {
    fn prepare_history(
        &self,
        conversation: &mut Conversation,
        agent: &str,
        content: &str,
        sender: &str,
    ) {
        let content = self
            .env
            .hook()
            .preprocess(agent, content)
            .unwrap_or_else(|| content.to_owned());
        if sender.is_empty() {
            conversation.history.push(HistoryEntry::user(&content));
        } else {
            conversation
                .history
                .push(HistoryEntry::user_with_sender(&content, sender));
        }

        conversation.history.retain(|e| !e.auto_injected);

        // Guest agent framing — auto-injected so it refreshes per turn.
        // Local instructions (e.g. `Crab.md`) used to be injected here
        // too but moved client-side: clients render them into `content`
        // before sending.
        if conversation.history.iter().any(|e| !e.agent.is_empty()) {
            let framing = HistoryEntry::user(
                "Messages wrapped in <from agent=\"...\"> tags are from guest agents \
                 who were consulted in this conversation. Continue responding as yourself."
                    .to_string(),
            )
            .auto_injected();
            let insert_pos = conversation.history.len().saturating_sub(1);
            conversation.history.insert(insert_pos, framing);
        }
    }

    pub async fn send_to(
        &self,
        conversation_id: u64,
        content: &str,
        sender: &str,
        tool_choice: Option<ToolChoice>,
        extra_tools: Vec<crabllm_core::Tool>,
    ) -> Result<AgentResponse> {
        let (agent_name, created_by, conversation_mutex) = self
            .acquire_slot(conversation_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("conversation {conversation_id} not found"))?;

        let mut conversation = conversation_mutex.lock().await;
        let pre_run_len = conversation.history.len();
        self.prepare_history(&mut conversation, &agent_name, content, sender);
        let mut agent = self
            .resolve_agent(&agent_name)
            .await
            .ok_or_else(|| anyhow::anyhow!("agent '{}' not registered", agent_name))?;
        agent.extend_tools(extra_tools);

        let (tx, mut rx) = mpsc::unbounded_channel();
        let response = agent
            .run(&mut conversation.history, tx, None, tool_choice)
            .await;

        let mut event_trace: Vec<wcore::EventLine> = Vec::new();
        while let Ok(event) = rx.try_recv() {
            self.env
                .hook()
                .on_event(&agent_name, conversation_id, &event);
            self.env
                .on_agent_event(&agent_name, conversation_id, &event);
            if let Some(line) = wcore::EventLine::from_agent_event(&event) {
                event_trace.push(line);
            }
        }

        self.finalize_run(
            &mut conversation,
            &agent_name,
            &created_by,
            pre_run_len,
            &event_trace,
        )
        .await;
        Ok(response)
    }

    pub fn stream_to(
        &self,
        conversation_id: u64,
        content: &str,
        sender: &str,
        tool_choice: Option<ToolChoice>,
        extra_tools: Vec<crabllm_core::Tool>,
    ) -> impl Stream<Item = AgentEvent> + '_ {
        let content = content.to_owned();
        let sender = sender.to_owned();
        stream! {
            let Some((agent_name, created_by, conversation_mutex)) =
                self.acquire_slot(conversation_id).await
            else {
                yield AgentEvent::Done(AgentResponse::error(
                    format!("conversation {conversation_id} not found"),
                ));
                return;
            };

            let mut conversation = conversation_mutex.lock().await;
            let pre_run_len = conversation.history.len();
            self.prepare_history(&mut conversation, &agent_name, &content, &sender);
            let Some(mut agent) = self.resolve_agent(&agent_name).await else {
                yield AgentEvent::Done(AgentResponse::error(
                    format!("agent '{}' not registered", agent_name),
                ));
                return;
            };
            agent.extend_tools(extra_tools);

            let (steer_tx, steer_rx) = watch::channel(None::<String>);
            self.steering.write().await.insert(conversation_id, steer_tx);
            let mut done_event: Option<AgentEvent> = None;
            let mut event_trace: Vec<wcore::EventLine> = Vec::new();
            {
                let mut event_stream = std::pin::pin!(agent.run_stream(&mut conversation.history, Some(conversation_id), Some(steer_rx), tool_choice));
                while let Some(event) = event_stream.next().await {
                    self.env.hook().on_event(&agent_name, conversation_id, &event);
                    self.env.on_agent_event(&agent_name, conversation_id, &event);
                    if let Some(line) = wcore::EventLine::from_agent_event(&event) {
                        event_trace.push(line);
                    }
                    if matches!(event, AgentEvent::Done(_)) {
                        done_event = Some(event);
                    } else {
                        yield event;
                    }
                }
            }
            self.steering.write().await.remove(&conversation_id);
            self.finalize_run(
                &mut conversation,
                &agent_name,
                &created_by,
                pre_run_len,
                &event_trace,
            )
            .await;
            if let Some(event) = done_event {
                yield event;
            }
        }
    }

    pub fn guest_stream_to(
        &self,
        conversation_id: u64,
        content: &str,
        sender: &str,
        guest: &str,
    ) -> impl Stream<Item = AgentEvent> + '_ {
        let content = content.to_owned();
        let sender = sender.to_owned();
        let guest = guest.to_owned();
        stream! {
            let Some(guest_agent) = self.resolve_agent(&guest).await else {
                yield AgentEvent::Done(AgentResponse::error(
                    format!("guest agent '{guest}' not registered"),
                ));
                return;
            };

            let Some((agent_name, created_by, conversation_mutex)) =
                self.acquire_slot(conversation_id).await
            else {
                yield AgentEvent::Done(AgentResponse::error(
                    format!("conversation {conversation_id} not found"),
                ));
                return;
            };

            let mut conversation = conversation_mutex.lock().await;
            let pre_run_len = conversation.history.len();

            let content = self
                .env
                .hook()
                .preprocess(&agent_name, &content)
                .unwrap_or_else(|| content.clone());
            if sender.is_empty() {
                conversation.history.push(HistoryEntry::user(&content));
            } else {
                conversation
                    .history
                    .push(HistoryEntry::user_with_sender(&content, &sender));
            }

            conversation.history.retain(|e| !e.auto_injected);

            let framing = HistoryEntry::system(format!(
                "You are joining a conversation as a guest. The primary agent is '{}'. \
                 Messages wrapped in <from agent=\"...\"> tags are from other agents. \
                 Respond as yourself to the user's latest message.",
                agent_name
            ))
            .auto_injected();
            let insert_pos = conversation.history.len().saturating_sub(1);
            conversation.history.insert(insert_pos, framing);

            let model_name = guest_agent.config.model.clone();

            let system = if guest_agent.config.system_prompt.is_empty() {
                None
            } else {
                Some(AnthropicSystem::Text(guest_agent.config.system_prompt.clone()))
            };

            let messages: Vec<AnthropicMessage> = conversation
                .history
                .iter()
                .map(|e| {
                    let msg = e.to_wire_message();
                    AnthropicMessage {
                        role: msg.role.as_str().to_string(),
                        content: AnthropicContent::Blocks(msg.content),
                    }
                })
                .collect();

            let max_tokens = DEFAULT_MAX_TOKENS;
            let thinking = guest_agent.config.thinking.then(|| ThinkingConfig {
                kind: "enabled".to_string(),
                budget_tokens: Some(max_tokens.saturating_sub(1)),
            });

            let request = AnthropicRequest {
                model: model_name.clone(),
                messages,
                max_tokens,
                system,
                temperature: None,
                top_p: None,
                stream: None,
                tools: None,
                tool_choice: None,
                stop_sequences: None,
                thinking,
            };

            let mut response_text = String::new();
            let mut reasoning = String::new();
            {
                let mut stream = std::pin::pin!(self.model.stream(request));
                while let Some(result) = stream.next().await {
                    match result {
                        Ok(chunk) => {
                            if let Some(text) = chunk.content() {
                                response_text.push_str(text);
                                yield AgentEvent::TextDelta(text.to_string());
                            }
                            if let Some(text) = chunk.reasoning_content() {
                                reasoning.push_str(text);
                                yield AgentEvent::ThinkingDelta(text.to_string());
                            }
                        }
                        Err(e) => {
                            yield AgentEvent::Done(AgentResponse {
                                final_response: None,
                                iterations: 1,
                                stop_reason: AgentStopReason::Error(e.to_string()),
                                steps: vec![],
                                model: model_name.clone(),
                            });
                            return;
                        }
                    }
                }
            }

            let reasoning = if reasoning.is_empty() {
                None
            } else {
                Some(reasoning)
            };
            let mut response_entry = HistoryEntry::assistant(&response_text, reasoning, None);
            response_entry.agent = guest.clone();
            conversation.history.push(response_entry);

            self.finalize_run(
                &mut conversation,
                &agent_name,
                &created_by,
                pre_run_len,
                &[],
            )
            .await;

            yield AgentEvent::Done(AgentResponse {
                final_response: Some(response_text),
                iterations: 1,
                stop_reason: AgentStopReason::TextResponse,
                steps: vec![],
                model: model_name,
            });
        }
    }
}
