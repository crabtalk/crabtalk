//! Execution — message sending and streaming through agents.

use super::Runtime;
use crate::{Config, Conversation, Env, Hook};
use anyhow::Result;
use async_stream::stream;
use crabllm_core::{ChatCompletionRequest, Message, ToolChoice};
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

        let mut recall_msgs =
            self.env
                .hook()
                .on_before_run(agent, conversation.id, &conversation.history);

        // Layered instructions (Crab.md).
        let cwd = self.env.effective_cwd(conversation.id);
        if let Some(instructions) = self.env.discover_instructions(&cwd) {
            recall_msgs.push(
                HistoryEntry::user(format!("<instructions>\n{instructions}\n</instructions>"))
                    .auto_injected(),
            );
        }

        // Guest agent framing.
        if conversation.history.iter().any(|e| !e.agent.is_empty()) {
            recall_msgs.push(
                HistoryEntry::user(
                    "Messages wrapped in <from agent=\"...\"> tags are from guest agents \
                     who were consulted in this conversation. Continue responding as yourself."
                        .to_string(),
                )
                .auto_injected(),
            );
        }
        if !recall_msgs.is_empty() {
            let insert_pos = conversation.history.len().saturating_sub(1);
            for (i, entry) in recall_msgs.into_iter().enumerate() {
                conversation.history.insert(insert_pos + i, entry);
            }
        }
    }

    pub async fn send_to(
        &self,
        conversation_id: u64,
        content: &str,
        sender: &str,
        tool_choice: Option<ToolChoice>,
    ) -> Result<AgentResponse> {
        let (agent_name, created_by, conversation_mutex) = self
            .acquire_slot(conversation_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("conversation {conversation_id} not found"))?;

        let mut conversation = conversation_mutex.lock().await;
        let pre_run_len = conversation.history.len();
        self.prepare_history(&mut conversation, &agent_name, content, sender);
        let agent = self
            .resolve_agent(&agent_name)
            .await
            .ok_or_else(|| anyhow::anyhow!("agent '{}' not registered", agent_name))?;

        let (tx, mut rx) = mpsc::unbounded_channel();
        let response = agent
            .run(&mut conversation.history, tx, None, tool_choice)
            .await;

        let mut compact_summary: Option<String> = None;
        while let Ok(event) = rx.try_recv() {
            if let AgentEvent::Compact { ref summary } = event {
                compact_summary = Some(summary.clone());
            }
            self.env
                .hook()
                .on_event(&agent_name, conversation_id, &event);
            self.env
                .on_agent_event(&agent_name, conversation_id, &event);
        }

        self.finalize_run(
            conversation_id,
            &mut conversation,
            conversation_mutex.clone(),
            &agent_name,
            &created_by,
            pre_run_len,
            compact_summary,
            &[],
        );
        Ok(response)
    }

    pub fn stream_to(
        &self,
        conversation_id: u64,
        content: &str,
        sender: &str,
        tool_choice: Option<ToolChoice>,
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
            let Some(agent) = self.resolve_agent(&agent_name).await else {
                yield AgentEvent::Done(AgentResponse::error(
                    format!("agent '{}' not registered", agent_name),
                ));
                return;
            };

            let (steer_tx, steer_rx) = watch::channel(None::<String>);
            self.steering.write().await.insert(conversation_id, steer_tx);
            let mut compact_summary: Option<String> = None;
            let mut done_event: Option<AgentEvent> = None;
            let mut event_trace: Vec<wcore::EventLine> = Vec::new();
            {
                let mut event_stream = std::pin::pin!(agent.run_stream(&mut conversation.history, Some(conversation_id), Some(steer_rx), tool_choice));
                while let Some(event) = event_stream.next().await {
                    if let AgentEvent::Compact { ref summary } = event {
                        compact_summary = Some(summary.clone());
                    }
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
                conversation_id,
                &mut conversation,
                conversation_mutex.clone(),
                &agent_name,
                &created_by,
                pre_run_len,
                compact_summary,
                &event_trace,
            );
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

            let mut messages = Vec::with_capacity(1 + conversation.history.len());
            if !guest_agent.config.system_prompt.is_empty() {
                messages.push(Message::system(&guest_agent.config.system_prompt));
            }
            messages.extend(conversation.history.iter().map(|e| e.to_wire_message()));

            let request = ChatCompletionRequest {
                model: model_name.clone(),
                messages,
                temperature: None,
                top_p: None,
                max_tokens: None,
                stream: None,
                stop: None,
                tools: None,
                tool_choice: None,
                frequency_penalty: None,
                presence_penalty: None,
                seed: None,
                user: None,
                reasoning_effort: if guest_agent.config.thinking {
                    Some("high".to_string())
                } else {
                    None
                },
                thinking: None,
                anthropic_max_tokens: None,
                extra: Default::default(),
            };

            let mut response_text = String::new();
            let mut reasoning = String::new();
            {
                let mut stream = std::pin::pin!(self.model.stream_ct(request));
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
                conversation_id,
                &mut conversation,
                conversation_mutex.clone(),
                &agent_name,
                &created_by,
                pre_run_len,
                None,
                &[],
            );

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
