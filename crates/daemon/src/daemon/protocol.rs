//! Server trait implementation for the Daemon.

use crate::daemon::Daemon;
use anyhow::{Context, Result};
use futures_util::{StreamExt, pin_mut};
use std::sync::Arc;
use wcore::AgentEvent;
use wcore::protocol::{
    api::Server,
    message::{
        DownloadEvent, DownloadInfo, HubAction, SendMsg, SendResponse, SessionInfo, StreamChunk,
        StreamEnd, StreamEvent, StreamMsg, StreamStart, StreamThinking, TaskEvent, TaskInfo,
        ToolCallInfo, ToolResultEvent, ToolStartEvent, ToolsCompleteEvent, stream_event,
    },
};

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
                    AgentEvent::Compact { .. } => {
                        // Compact events are handled by on_event in the hook layer.
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
            .map(crate::hook::system::task::TaskRegistry::task_info)
            .collect())
    }

    async fn kill_task(&self, task_id: u64) -> Result<bool> {
        use crate::hook::system::task::TaskStatus;
        let rt = self.runtime.read().await.clone();
        let tasks = rt.hook.tasks.clone();
        let mut registry = tasks.lock().await;
        let Some(task) = registry.get(task_id) else {
            return Ok(false);
        };
        match task.status {
            TaskStatus::InProgress | TaskStatus::Blocked => {
                let session_id = task.session_id;
                registry.kill(task_id);
                crate::hook::system::task::tool::try_promote(
                    &mut registry,
                    tasks.clone(),
                    rt.hook.event_tx.clone(),
                    rt.hook.task_timeout,
                );
                // Close associated session outside the lock.
                if let Some(sid) = session_id {
                    drop(registry);
                    rt.close_session(sid).await;
                }
                Ok(true)
            }
            TaskStatus::Queued => {
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

    fn hub(
        &self,
        package: String,
        action: HubAction,
        filters: Vec<String>,
    ) -> impl futures_core::Stream<Item = Result<DownloadEvent>> + Send {
        let runtime = self.runtime.clone();
        async_stream::try_stream! {
            let rt = runtime.read().await.clone();
            let registry = rt.hook.downloads.clone();
            let package = compact_str::CompactString::from(package.as_str());
            match action {
                HubAction::Install => {
                    let s = crate::ext::hub::package::install(package, registry, filters);
                    pin_mut!(s);
                    while let Some(event) = s.next().await {
                        yield event?;
                    }
                }
                HubAction::Uninstall => {
                    let s = crate::ext::hub::package::uninstall(package, registry, filters);
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

    async fn service_query(&self, service: String, query: String) -> Result<String> {
        let rt = self.runtime.read().await.clone();
        let registry = rt
            .hook
            .registry
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no service registry"))?;
        let handle = registry
            .query
            .get(&service)
            .ok_or_else(|| anyhow::anyhow!("service '{}' not available", service))?;
        let req = wcore::protocol::ext::ExtRequest {
            msg: Some(wcore::protocol::ext::ext_request::Msg::ServiceQuery(
                wcore::protocol::ext::ExtServiceQuery { query },
            )),
        };
        let resp = handle.request(&req).await?;
        match resp.msg {
            Some(wcore::protocol::ext::ext_response::Msg::ServiceQueryResult(result)) => {
                Ok(result.result)
            }
            Some(wcore::protocol::ext::ext_response::Msg::Error(e)) => {
                anyhow::bail!("service '{}' error: {}", service, e.message)
            }
            other => anyhow::bail!("unexpected response from service '{}': {other:?}", service),
        }
    }

    async fn get_service_schema(&self, service: String) -> Result<String> {
        let rt = self.runtime.read().await.clone();
        let registry = rt
            .hook
            .registry
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no service registry"))?;
        let handle = registry
            .query
            .get(&service)
            .or_else(|| registry.tools.values().find(|h| h.name.as_str() == service))
            .ok_or_else(|| anyhow::anyhow!("service '{}' not found", service))?;
        let req = wcore::protocol::ext::ExtRequest {
            msg: Some(wcore::protocol::ext::ext_request::Msg::GetSchema(
                wcore::protocol::ext::ExtGetSchema {},
            )),
        };
        let resp = handle.request(&req).await?;
        match resp.msg {
            Some(wcore::protocol::ext::ext_response::Msg::SchemaResult(result)) => {
                Ok(result.schema)
            }
            Some(wcore::protocol::ext::ext_response::Msg::Error(e)) => {
                anyhow::bail!("service '{}' schema error: {}", service, e.message)
            }
            other => anyhow::bail!(
                "unexpected schema response from service '{}': {other:?}",
                service
            ),
        }
    }

    async fn get_all_schemas(&self) -> Result<std::collections::HashMap<String, String>> {
        let rt = self.runtime.read().await.clone();
        let registry = rt
            .hook
            .registry
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no service registry"))?;
        let mut schemas = std::collections::HashMap::new();
        // Collect unique service handles from the query registry.
        for (name, handle) in &registry.query {
            let req = wcore::protocol::ext::ExtRequest {
                msg: Some(wcore::protocol::ext::ext_request::Msg::GetSchema(
                    wcore::protocol::ext::ExtGetSchema {},
                )),
            };
            if let Ok(resp) = handle.request(&req).await
                && let Some(wcore::protocol::ext::ext_response::Msg::SchemaResult(result)) =
                    resp.msg
            {
                schemas.insert(name.clone(), result.schema);
            }
        }
        Ok(schemas)
    }

    async fn list_services(&self) -> Result<Vec<wcore::protocol::message::ServiceInfoMsg>> {
        let rt = self.runtime.read().await.clone();
        let registry = rt.hook.registry.as_ref();
        let mut services = Vec::new();
        if let Some(reg) = registry {
            // Collect unique service names from capability buckets.
            let mut seen = std::collections::HashSet::new();
            let all_handles: Vec<_> = reg.query.values().chain(reg.tools.values()).collect();
            for handle in all_handles {
                let name = handle.name.to_string();
                if !seen.insert(name.clone()) {
                    continue;
                }
                let capabilities: Vec<String> = handle
                    .capabilities
                    .iter()
                    .filter_map(|c| match &c.cap {
                        Some(wcore::protocol::ext::capability::Cap::Tools(_)) => {
                            Some("tools".into())
                        }
                        Some(wcore::protocol::ext::capability::Cap::Query(_)) => {
                            Some("query".into())
                        }
                        _ => None,
                    })
                    .collect();
                services.push(wcore::protocol::message::ServiceInfoMsg {
                    name,
                    kind: "extension".into(),
                    status: "running".into(),
                    capabilities,
                    has_config: true,
                });
            }
        }
        Ok(services)
    }

    async fn set_service_config(&self, service: String, config: String) -> Result<()> {
        let mut daemon_config = self.load_config()?;
        let svc = daemon_config
            .services
            .get_mut(&service)
            .ok_or_else(|| anyhow::anyhow!("service '{}' not found in config", service))?;
        let parsed: serde_json::Value =
            serde_json::from_str(&config).context("invalid service config JSON")?;
        svc.config = parsed;
        let toml_str =
            toml::to_string_pretty(&daemon_config).context("failed to serialize config to TOML")?;
        let config_path = self.config_dir.join("walrus.toml");
        std::fs::write(&config_path, toml_str)
            .with_context(|| format!("failed to write {}", config_path.display()))?;
        self.reload().await
    }

    async fn reload(&self) -> Result<()> {
        self.reload().await
    }
}

impl Daemon {
    /// Load the current `DaemonConfig` from disk.
    fn load_config(&self) -> Result<crate::DaemonConfig> {
        crate::DaemonConfig::load(&self.config_dir.join("walrus.toml"))
    }
}
