//! Server trait implementation for the Daemon.

use crate::daemon::Daemon;
use anyhow::{Context, Result};
use compact_str::CompactString;
use futures_util::{StreamExt, pin_mut};
use std::sync::Arc;
use wcore::protocol::{
    api::Server,
    message::{
        DownloadEvent, DownloadRequest, HubAction, Resource, SendRequest, SendResponse,
        StreamEvent, StreamRequest, TaskEvent,
        server::{DownloadInfo, ResourceList, SessionInfo, SkillInfo, TaskInfo, ToolCallInfo},
    },
};
use wcore::{AgentEvent, model::Model};

impl Server for Daemon {
    async fn send(&self, req: SendRequest) -> Result<SendResponse> {
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
        req: StreamRequest,
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

            yield StreamEvent::Start { agent: agent.clone(), session: session_id };

            let stream = rt.stream_to(session_id, &content, &sender);
            pin_mut!(stream);
            while let Some(event) = stream.next().await {
                match event {
                    AgentEvent::TextDelta(text) => {
                        yield StreamEvent::Chunk { content: text };
                    }
                    AgentEvent::ThinkingDelta(text) => {
                        yield StreamEvent::Thinking { content: text };
                    }
                    AgentEvent::ToolCallsStart(calls) => {
                        yield StreamEvent::ToolStart {
                            calls: calls.into_iter().map(|c| ToolCallInfo {
                                name: CompactString::from(c.function.name.as_str()),
                                arguments: c.function.arguments,
                            }).collect(),
                        };
                    }
                    AgentEvent::ToolResult { call_id, output } => {
                        yield StreamEvent::ToolResult { call_id, output };
                    }
                    AgentEvent::ToolCallsComplete => {
                        yield StreamEvent::ToolsComplete;
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

            yield StreamEvent::End { agent: agent.clone() };
        }
    }

    fn download(
        &self,
        req: DownloadRequest,
    ) -> impl futures_core::Stream<Item = Result<DownloadEvent>> + Send {
        let runtime = self.runtime.clone();
        async_stream::try_stream! {
            let rt = runtime.read().await.clone();
            let registry = rt.hook.downloads.clone();
            let s = crate::ext::hub::model::download(req.model, registry);
            pin_mut!(s);
            while let Some(event) = s.next().await {
                yield event?;
            }
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
                agent: s.agent.clone(),
                created_by: s.created_by.clone(),
                message_count: s.history.len(),
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
                agent: t.agent.clone(),
                status: t.status.to_string(),
                description: t.description.clone(),
                result: t.result.clone(),
                error: t.error.clone(),
                created_by: t.created_by.clone(),
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

    async fn evaluate(&self, req: SendRequest) -> Result<bool> {
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
        package: CompactString,
        action: HubAction,
    ) -> impl futures_core::Stream<Item = Result<DownloadEvent>> + Send {
        let runtime = self.runtime.clone();
        async_stream::try_stream! {
            let rt = runtime.read().await.clone();
            let registry = rt.hook.downloads.clone();
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

    async fn list_resource(&self, resource: Resource) -> Result<ResourceList> {
        match resource {
            Resource::Mcp => {
                let config = self.load_config()?;
                let mcps = config
                    .mcps
                    .into_iter()
                    .map(|(k, v)| (CompactString::from(k), v))
                    .collect();
                Ok(ResourceList::Mcps(mcps))
            }
            Resource::Skill => {
                let rt = self.runtime.read().await.clone();
                let registry = rt.hook.skills.registry.lock().await;
                let skills = registry
                    .skills()
                    .into_iter()
                    .map(|s| SkillInfo {
                        name: s.name.clone(),
                        description: s.description.clone(),
                        license: s.license.clone(),
                        compatibility: s.compatibility.clone(),
                    })
                    .collect();
                Ok(ResourceList::Skills(skills))
            }
            Resource::Agent => {
                let config = self.load_config()?;
                let agents = config
                    .agents
                    .into_iter()
                    .map(|(k, v)| (CompactString::from(k), v))
                    .collect();
                Ok(ResourceList::Agents(agents))
            }
            Resource::Provider => {
                let config = self.load_config()?;
                let providers = config.model.providers.into_iter().collect();
                Ok(ResourceList::Providers(providers))
            }
        }
    }

    async fn add_resource(&self, resource: Resource, name: String, value: String) -> Result<()> {
        match resource {
            Resource::Skill => anyhow::bail!("skills are read-only; use hub install"),
            Resource::Mcp => {
                let typed: wcore::McpServerConfig =
                    serde_json::from_str(&value).context("invalid McpServerConfig JSON")?;
                self.edit_config(|doc| {
                    let table = doc
                        .entry("mcps")
                        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
                        .as_table_mut()
                        .context("mcps is not a table")?;
                    let ser = toml_edit::ser::to_document(&typed)
                        .context("failed to serialize McpServerConfig")?;
                    table.insert(&name, toml_edit::Item::Table(ser.as_table().clone()));
                    Ok(())
                })?;
                self.reload().await
            }
            Resource::Agent => {
                let typed: wcore::AgentConfig =
                    serde_json::from_str(&value).context("invalid AgentConfig JSON")?;
                self.edit_config(|doc| {
                    let table = doc
                        .entry("agents")
                        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
                        .as_table_mut()
                        .context("agents is not a table")?;
                    let ser = toml_edit::ser::to_document(&typed)
                        .context("failed to serialize AgentConfig")?;
                    table.insert(&name, toml_edit::Item::Table(ser.as_table().clone()));
                    Ok(())
                })?;
                self.reload().await
            }
            Resource::Provider => {
                let typed: wcore::ProviderConfig =
                    serde_json::from_str(&value).context("invalid ProviderConfig JSON")?;
                self.edit_config(|doc| {
                    let model = doc
                        .entry("model")
                        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
                        .as_table_mut()
                        .context("model is not a table")?;
                    let providers = model
                        .entry("providers")
                        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
                        .as_table_mut()
                        .context("model.providers is not a table")?;
                    let ser = toml_edit::ser::to_document(&typed)
                        .context("failed to serialize ProviderConfig")?;
                    providers.insert(&name, toml_edit::Item::Table(ser.as_table().clone()));
                    Ok(())
                })?;
                self.reload().await
            }
        }
    }

    async fn remove_resource(&self, resource: Resource, name: String) -> Result<()> {
        match resource {
            Resource::Skill => anyhow::bail!("skills are read-only; use hub uninstall"),
            Resource::Mcp => {
                self.edit_config(|doc| {
                    if let Some(table) = doc.get_mut("mcps").and_then(|v| v.as_table_mut()) {
                        table.remove(&name);
                    }
                    Ok(())
                })?;
                self.reload().await
            }
            Resource::Agent => {
                self.edit_config(|doc| {
                    if let Some(table) = doc.get_mut("agents").and_then(|v| v.as_table_mut()) {
                        table.remove(&name);
                    }
                    Ok(())
                })?;
                self.reload().await
            }
            Resource::Provider => {
                self.edit_config(|doc| {
                    if let Some(model) = doc.get_mut("model").and_then(|v| v.as_table_mut())
                        && let Some(providers) =
                            model.get_mut("providers").and_then(|v| v.as_table_mut())
                    {
                        providers.remove(&name);
                    }
                    Ok(())
                })?;
                self.reload().await
            }
        }
    }
}

impl Daemon {
    /// Load the current `DaemonConfig` from disk.
    fn load_config(&self) -> Result<crate::DaemonConfig> {
        crate::DaemonConfig::load(&self.config_dir.join("walrus.toml"))
    }

    /// Read `walrus.toml`, apply an edit function, and write back.
    fn edit_config(&self, f: impl FnOnce(&mut toml_edit::DocumentMut) -> Result<()>) -> Result<()> {
        let config_path = self.config_dir.join("walrus.toml");
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("cannot read {}", config_path.display()))?;
        let mut doc: toml_edit::DocumentMut = content
            .parse()
            .with_context(|| format!("invalid TOML in {}", config_path.display()))?;
        f(&mut doc)?;
        std::fs::write(&config_path, doc.to_string())
            .with_context(|| format!("failed to write {}", config_path.display()))?;
        Ok(())
    }
}
