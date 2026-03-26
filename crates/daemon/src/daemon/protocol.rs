//! Server trait implementation for the Daemon.

use crate::daemon::Daemon;
use anyhow::{Context, Result};
use futures_util::{StreamExt, pin_mut};
use std::sync::Arc;
use wcore::AgentEvent;
use wcore::protocol::{
    api::Server,
    message::{
        AgentEventMsg, AskOption, AskQuestion, AskUserEvent, SendMsg, SendResponse, SessionInfo,
        StreamChunk, StreamEnd, StreamEvent, StreamMsg, StreamStart, StreamThinking, ToolCallInfo,
        ToolResultEvent, ToolStartEvent, ToolsCompleteEvent, stream_event,
    },
};

impl Server for Daemon {
    async fn send(&self, req: SendMsg) -> Result<SendResponse> {
        let rt: Arc<_> = self.runtime.read().await.clone();
        let sender = req.sender.as_deref().unwrap_or("");
        let created_by = if sender.is_empty() { "user" } else { sender };
        let cwd = req.cwd.map(std::path::PathBuf::from);
        let session_id = match req.session {
            Some(id) => id,
            None => {
                let id = if let Some(ref file) = req.resume_file {
                    rt.load_specific_session(std::path::Path::new(file)).await?
                } else if req.new_chat {
                    rt.create_session(&req.agent, created_by).await?
                } else {
                    rt.get_or_create_session(&req.agent, created_by).await?
                };
                if let Some(ref cwd) = cwd {
                    rt.hook
                        .bridge
                        .session_cwds
                        .lock()
                        .await
                        .insert(id, cwd.clone());
                }
                id
            }
        };
        let response = rt.send_to(session_id, &req.content, sender).await?;
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
        let cwd = req.cwd.map(std::path::PathBuf::from);
        let new_chat = req.new_chat;
        let resume_file = req.resume_file;
        async_stream::try_stream! {
            let rt: Arc<_> = runtime.read().await.clone();
            let created_by = if sender.is_empty() { "user".into() } else { sender.clone() };
            let session_id = match req_session {
                Some(id) => id,
                None => {
                    let id = if let Some(ref file) = resume_file {
                        rt.load_specific_session(std::path::Path::new(file)).await?
                    } else if new_chat {
                        rt.create_session(&agent, created_by.as_str()).await?
                    } else {
                        rt.get_or_create_session(&agent, created_by.as_str()).await?
                    };
                    if let Some(ref cwd) = cwd {
                        rt.hook.bridge.session_cwds.lock().await.insert(id, cwd.clone());
                    }
                    id
                }
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
                    AgentEvent::ToolCallsBegin(calls) => {
                        yield StreamEvent { event: Some(stream_event::Event::ToolStart(ToolStartEvent {
                            calls: calls.into_iter().map(|c| ToolCallInfo {
                                name: c.function.name.to_string(),
                                arguments: String::new(),
                            }).collect(),
                        })) };
                    }
                    AgentEvent::ToolCallsStart(calls) => {
                        // Extract structured questions from ask_user calls.
                        let ask_questions: Vec<AskQuestion> = calls
                            .iter()
                            .filter(|c| c.function.name == "ask_user")
                            .filter_map(|c| {
                                serde_json::from_str::<runtime::ask_user::AskUser>(&c.function.arguments)
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
                    AgentEvent::ToolResult { call_id, output } => {
                        yield StreamEvent { event: Some(stream_event::Event::ToolResult(ToolResultEvent { call_id: call_id.to_string(), output })) };
                    }
                    AgentEvent::ToolCallsComplete => {
                        yield StreamEvent { event: Some(stream_event::Event::ToolsComplete(ToolsCompleteEvent {})) };
                    }
                    AgentEvent::Compact { .. } => {
                    }
                    AgentEvent::Done(resp) => {
                        let error = if let wcore::AgentStopReason::Error(e) = resp.stop_reason {
                            e
                        } else {
                            String::new()
                        };
                        yield StreamEvent { event: Some(stream_event::Event::End(StreamEnd { agent: agent.clone(), error })) };
                        return;
                    }
                }
            }
            yield StreamEvent { event: Some(stream_event::Event::End(StreamEnd { agent: agent.clone(), error: String::new() })) };
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
            let active = rt.is_active(s.id).await;
            infos.push(SessionInfo {
                id: s.id,
                agent: s.agent.to_string(),
                created_by: s.created_by.to_string(),
                message_count: s.history.len() as u64,
                alive_secs: s.uptime_secs,
                active,
                title: s.title.clone(),
            });
        }
        Ok(infos)
    }

    async fn kill_session(&self, session: u64) -> Result<bool> {
        let rt = self.runtime.read().await.clone();
        rt.hook.bridge.pending_asks.lock().await.remove(&session);
        rt.hook.bridge.session_cwds.lock().await.remove(&session);
        Ok(rt.close_session(session).await)
    }

    fn subscribe_events(&self) -> impl futures_core::Stream<Item = Result<AgentEventMsg>> + Send {
        let runtime = self.runtime.clone();
        async_stream::try_stream! {
            let rt = runtime.read().await.clone();
            let mut rx = rt.hook.bridge.subscribe_events();
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
        let config_path = self.config_dir.join(wcore::paths::CONFIG_FILE);
        std::fs::write(&config_path, toml_str)
            .with_context(|| format!("failed to write {}", config_path.display()))?;
        self.reload().await
    }

    async fn reload(&self) -> Result<()> {
        self.reload().await
    }

    async fn reply_to_ask(&self, session: u64, content: String) -> Result<()> {
        let rt = self.runtime.read().await.clone();
        if let Some(tx) = rt.hook.bridge.pending_asks.lock().await.remove(&session) {
            let _ = tx.send(content);
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Some(tx) = rt.hook.bridge.pending_asks.lock().await.remove(&session) {
            let _ = tx.send(content);
            return Ok(());
        }
        anyhow::bail!("no pending ask_user for session {session}")
    }
}

impl Daemon {
    /// Load the current `DaemonConfig` from disk.
    fn load_config(&self) -> Result<crate::DaemonConfig> {
        crate::DaemonConfig::load(&self.config_dir.join(wcore::paths::CONFIG_FILE))
    }
}
