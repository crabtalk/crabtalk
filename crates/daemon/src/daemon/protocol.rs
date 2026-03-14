//! Server trait implementation for the Daemon.

use crate::daemon::Daemon;
use anyhow::{Context, Result};
use futures_util::{StreamExt, pin_mut};
use std::sync::Arc;
use wcore::protocol::{
    api::Server,
    message::{
        DownloadEvent, DownloadInfo, EntityInfo, EntityList, HubAction, JournalInfo, JournalList,
        MemoryOp, MemoryResult, RelationInfo, RelationList, SendMsg, SendResponse, SessionInfo,
        StreamChunk, StreamEnd, StreamEvent, StreamMsg, StreamStart, StreamThinking, TaskEvent,
        TaskInfo, ToolCallInfo, ToolResultEvent, ToolStartEvent, ToolsCompleteEvent, memory_op,
        memory_result, stream_event,
    },
};
use wcore::{AgentEvent, model::Model};

impl Server for Daemon {
    async fn send(&self, req: SendMsg) -> Result<SendResponse> {
        let rt: Arc<_> = self.runtime.read().await.clone();
        let sender = req.sender.as_deref().unwrap_or("");
        let created_by = if sender.is_empty() { "user" } else { sender };
        let (session_id, is_new) = match req.session {
            Some(id) => (id, false),
            None => (rt.create_session(&req.agent, created_by).await?, true),
        };
        let response = rt.send_to(session_id, &req.content, sender).await?;
        if is_new {
            rt.close_session(session_id).await;
        }
        Ok(SendResponse {
            agent: req.agent,
            content: response.final_response.unwrap_or_default(),
            session: session_id,
        })
    }

    fn stream(
        &self,
        req: StreamMsg,
    ) -> impl futures_core::Stream<Item = Result<StreamEvent>> + Send {
        let runtime = self.runtime.clone();
        let agent = req.agent;
        let content = req.content;
        let req_session = req.session;
        let sender = req.sender.unwrap_or_default();
        async_stream::try_stream! {
            let rt: Arc<_> = runtime.read().await.clone();
            let created_by = if sender.is_empty() { "user".into() } else { sender.clone() };
            let (session_id, is_new) = match req_session {
                Some(id) => (id, false),
                None => (rt.create_session(&agent, created_by.as_str()).await?, true),
            };

            yield StreamEvent { event: Some(stream_event::Event::Start(StreamStart { agent: agent.clone(), session: session_id })) };

            let stream = rt.stream_to(session_id, &content, &sender);
            pin_mut!(stream);
            while let Some(event) = stream.next().await {
                match event {
                    AgentEvent::TextDelta(text) => {
                        yield StreamEvent { event: Some(stream_event::Event::Chunk(StreamChunk { content: text })) };
                    }
                    AgentEvent::ThinkingDelta(text) => {
                        yield StreamEvent { event: Some(stream_event::Event::Thinking(StreamThinking { content: text })) };
                    }
                    AgentEvent::ToolCallsStart(calls) => {
                        yield StreamEvent { event: Some(stream_event::Event::ToolStart(ToolStartEvent {
                            calls: calls.into_iter().map(|c| ToolCallInfo {
                                name: c.function.name.to_string(),
                                arguments: c.function.arguments,
                            }).collect(),
                        })) };
                    }
                    AgentEvent::ToolResult { call_id, output } => {
                        yield StreamEvent { event: Some(stream_event::Event::ToolResult(ToolResultEvent { call_id: call_id.to_string(), output })) };
                    }
                    AgentEvent::ToolCallsComplete => {
                        yield StreamEvent { event: Some(stream_event::Event::ToolsComplete(ToolsCompleteEvent {})) };
                    }
                    AgentEvent::Done(resp) => {
                        if let wcore::AgentStopReason::Error(e) = &resp.stop_reason {
                            if is_new {
                                rt.close_session(session_id).await;
                            }
                            Err(anyhow::anyhow!("{e}"))?;
                        }
                        break;
                    }
                }
            }
            if is_new {
                rt.close_session(session_id).await;
            }

            yield StreamEvent { event: Some(stream_event::Event::End(StreamEnd { agent: agent.clone() })) };
        }
    }

    async fn ping(&self) -> Result<()> {
        Ok(())
    }

    async fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        let rt = self.runtime.read().await.clone();
        let sessions = rt.sessions().await;
        let mut infos = Vec::with_capacity(sessions.len());
        for s in sessions {
            let s = s.lock().await;
            infos.push(SessionInfo {
                id: s.id,
                agent: s.agent.to_string(),
                created_by: s.created_by.to_string(),
                message_count: s.history.len() as u64,
                alive_secs: s.created_at.elapsed().as_secs(),
            });
        }
        Ok(infos)
    }

    async fn kill_session(&self, session: u64) -> Result<bool> {
        let rt = self.runtime.read().await.clone();
        Ok(rt.close_session(session).await)
    }

    async fn list_tasks(&self) -> Result<Vec<TaskInfo>> {
        let rt = self.runtime.read().await.clone();
        let registry = rt.hook.tasks.lock().await;
        let tasks = registry.list(None, None, None);
        Ok(tasks
            .into_iter()
            .map(|t| TaskInfo {
                id: t.id,
                parent_id: t.parent_id,
                agent: t.agent.to_string(),
                status: t.status.to_string(),
                description: t.description.clone(),
                result: t.result.clone(),
                error: t.error.clone(),
                created_by: t.created_by.to_string(),
                prompt_tokens: t.prompt_tokens,
                completion_tokens: t.completion_tokens,
                alive_secs: t.created_at.elapsed().as_secs(),
                blocked_on: t.blocked_on.as_ref().map(|i| i.question.clone()),
            })
            .collect())
    }

    async fn kill_task(&self, task_id: u64) -> Result<bool> {
        let rt = self.runtime.read().await.clone();
        let tasks = rt.hook.tasks.clone();
        let mut registry = tasks.lock().await;
        let Some(task) = registry.get(task_id) else {
            return Ok(false);
        };
        match task.status {
            crate::hook::task::TaskStatus::InProgress | crate::hook::task::TaskStatus::Blocked => {
                if let Some(handle) = &task.abort_handle {
                    handle.abort();
                }
                registry.set_status(task_id, crate::hook::task::TaskStatus::Failed);
                if let Some(task) = registry.get_mut(task_id) {
                    task.error = Some("killed by user".into());
                }
                // Close associated session.
                if let Some(sid) = registry.get(task_id).and_then(|t| t.session_id) {
                    drop(registry);
                    rt.close_session(sid).await;
                    let mut registry = tasks.lock().await;
                    registry.promote_next(tasks.clone());
                } else {
                    registry.promote_next(tasks.clone());
                }
                Ok(true)
            }
            crate::hook::task::TaskStatus::Queued => {
                registry.remove(task_id);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn approve_task(&self, task_id: u64, response: String) -> Result<bool> {
        let rt = self.runtime.read().await.clone();
        let mut registry = rt.hook.tasks.lock().await;
        Ok(registry.approve(task_id, response))
    }

    async fn evaluate(&self, req: SendMsg) -> Result<bool> {
        let rt: Arc<_> = self.runtime.read().await.clone();
        let agent = rt
            .get_agent(&req.agent)
            .ok_or_else(|| anyhow::anyhow!("agent '{}' not found", req.agent))?;

        let sender = req.sender.as_deref().unwrap_or("");

        // Build sender context from memory.
        let sender_context = if !sender.is_empty() {
            let query = format!("{sender} profile");
            let args = serde_json::json!({ "query": query, "entity_type": "profile", "limit": 3 });
            let recall_result = rt.hook.memory.dispatch_recall(&args.to_string()).await;
            if recall_result == "no entities found" {
                String::new()
            } else {
                recall_result
            }
        } else {
            String::new()
        };

        // Build a minimal evaluation prompt.
        let mut eval_prompt = String::from(
            "You are deciding whether to respond to a message in a group chat. \
             Reply with exactly \"yes\" or \"no\".\n\n",
        );
        if !sender_context.is_empty() {
            eval_prompt.push_str("Sender profile:\n");
            eval_prompt.push_str(&sender_context);
            eval_prompt.push('\n');
        }
        eval_prompt.push_str("Message: ");
        eval_prompt.push_str(&req.content);
        eval_prompt.push_str("\n\nShould you respond? (yes/no)");

        let model_name = agent
            .config
            .model
            .clone()
            .unwrap_or_else(|| rt.model.active_model());

        let messages = vec![
            wcore::model::Message::system(&agent.config.system_prompt),
            wcore::model::Message::user(eval_prompt),
        ];

        let request = wcore::model::Request::new(model_name).with_messages(messages);

        match rt.model.send(&request).await {
            Ok(response) => {
                let text = response.message().map(|m| m.content).unwrap_or_default();
                let lower = text.trim().to_lowercase();
                Ok(lower.starts_with("yes"))
            }
            Err(e) => {
                tracing::warn!(agent = %req.agent, "evaluate LLM call failed: {e}, defaulting to respond");
                Ok(true)
            }
        }
    }

    fn hub(
        &self,
        package: String,
        action: HubAction,
    ) -> impl futures_core::Stream<Item = Result<DownloadEvent>> + Send {
        let runtime = self.runtime.clone();
        async_stream::try_stream! {
            let rt = runtime.read().await.clone();
            let registry = rt.hook.downloads.clone();
            let package = compact_str::CompactString::from(package.as_str());
            match action {
                HubAction::Install => {
                    let s = crate::ext::hub::package::install(package, registry);
                    pin_mut!(s);
                    while let Some(event) = s.next().await {
                        yield event?;
                    }
                }
                HubAction::Uninstall => {
                    let s = crate::ext::hub::package::uninstall(package, registry);
                    pin_mut!(s);
                    while let Some(event) = s.next().await {
                        yield event?;
                    }
                }
            }
        }
    }

    fn subscribe_tasks(&self) -> impl futures_core::Stream<Item = Result<TaskEvent>> + Send {
        let runtime = self.runtime.clone();
        async_stream::try_stream! {
            let rt = runtime.read().await.clone();
            let mut rx = rt.hook.tasks.lock().await.subscribe();
            loop {
                match rx.recv().await {
                    Ok(event) => yield event,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        }
    }

    async fn list_downloads(&self) -> Result<Vec<DownloadInfo>> {
        let rt = self.runtime.read().await.clone();
        let registry = rt.hook.downloads.lock().await;
        Ok(registry.list())
    }

    fn subscribe_downloads(
        &self,
    ) -> impl futures_core::Stream<Item = Result<DownloadEvent>> + Send {
        let runtime = self.runtime.clone();
        async_stream::try_stream! {
            let rt = runtime.read().await.clone();
            let mut rx = rt.hook.downloads.lock().await.subscribe();
            loop {
                match rx.recv().await {
                    Ok(event) => yield event,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        }
    }

    async fn get_config(&self) -> Result<String> {
        let config = self.load_config()?;
        serde_json::to_string(&config).context("failed to serialize config")
    }

    async fn set_config(&self, config: String) -> Result<()> {
        let parsed: crate::DaemonConfig =
            serde_json::from_str(&config).context("invalid DaemonConfig JSON")?;
        let toml_str =
            toml::to_string_pretty(&parsed).context("failed to serialize config to TOML")?;
        let config_path = self.config_dir.join("walrus.toml");
        std::fs::write(&config_path, toml_str)
            .with_context(|| format!("failed to write {}", config_path.display()))?;
        self.reload().await
    }

    async fn memory_query(&self, query: MemoryOp) -> Result<MemoryResult> {
        let rt = self.runtime.read().await.clone();
        let lance = &rt.hook.memory.lance;
        let default_limit = 50u32;

        let op = query.op.ok_or_else(|| anyhow::anyhow!("empty memory op"))?;

        match op {
            memory_op::Op::Entities(req) => {
                let limit = req.limit.unwrap_or(default_limit) as usize;
                let entities = lance
                    .list_entities(req.entity_type.as_deref(), limit)
                    .await?;
                Ok(MemoryResult {
                    result: Some(memory_result::Result::Entities(EntityList {
                        entities: entities
                            .into_iter()
                            .map(|e| EntityInfo {
                                entity_type: e.entity_type,
                                key: e.key,
                                value: e.value,
                                created_at: e.created_at,
                            })
                            .collect(),
                    })),
                })
            }
            memory_op::Op::Relations(req) => {
                let limit = req.limit.unwrap_or(default_limit) as usize;
                let relations = lance
                    .list_relations(req.entity_id.as_deref(), limit)
                    .await?;
                Ok(MemoryResult {
                    result: Some(memory_result::Result::Relations(RelationList {
                        relations: relations
                            .into_iter()
                            .map(|r| RelationInfo {
                                source_id: r.source,
                                relation: r.relation,
                                target_id: r.target,
                                created_at: r.created_at,
                            })
                            .collect(),
                    })),
                })
            }
            memory_op::Op::Journals(req) => {
                let limit = req.limit.unwrap_or(default_limit) as usize;
                let journals = lance.list_journals(req.agent.as_deref(), limit).await?;
                Ok(MemoryResult {
                    result: Some(memory_result::Result::Journals(JournalList {
                        journals: journals
                            .into_iter()
                            .map(|j| JournalInfo {
                                summary: j.summary,
                                agent: j.agent,
                                created_at: j.created_at,
                            })
                            .collect(),
                    })),
                })
            }
            memory_op::Op::Search(req) => {
                let limit = req.limit.unwrap_or(default_limit) as usize;
                let entities = lance
                    .search_entities(&req.query, req.entity_type.as_deref(), limit)
                    .await?;
                Ok(MemoryResult {
                    result: Some(memory_result::Result::Entities(EntityList {
                        entities: entities
                            .into_iter()
                            .map(|e| EntityInfo {
                                entity_type: e.entity_type,
                                key: e.key,
                                value: e.value,
                                created_at: e.created_at,
                            })
                            .collect(),
                    })),
                })
            }
        }
    }
}

impl Daemon {
    /// Load the current `DaemonConfig` from disk.
    fn load_config(&self) -> Result<crate::DaemonConfig> {
        crate::DaemonConfig::load(&self.config_dir.join("walrus.toml"))
    }
}
