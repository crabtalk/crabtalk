//! Server trait implementation for the Daemon.

use crate::{daemon::Daemon, ext::hub};
use anyhow::Result;
use compact_str::CompactString;
use futures_util::{StreamExt, pin_mut};
use std::sync::Arc;
use wcore::AgentEvent;
use wcore::protocol::{
    api::Server,
    message::{
        DownloadEvent, DownloadRequest, HubAction, HubEvent, SendRequest, SendResponse,
        StreamEvent, StreamRequest,
        server::{SessionInfo, TaskInfo},
    },
};

impl Server for Daemon {
    async fn send(&self, req: SendRequest) -> Result<SendResponse> {
        let rt: Arc<_> = self.runtime.read().await.clone();
        let (session_id, is_new) = match req.session {
            Some(id) => (id, false),
            None => (rt.create_session(&req.agent, "user").await?, true),
        };
        let response = rt.send_to(session_id, &req.content).await?;
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
        async_stream::try_stream! {
            let rt: Arc<_> = runtime.read().await.clone();
            let (session_id, is_new) = match req_session {
                Some(id) => (id, false),
                None => (rt.create_session(&agent, "user").await?, true),
            };

            yield StreamEvent::Start { agent: agent.clone(), session: session_id };

            let stream = rt.stream_to(session_id, &content);
            pin_mut!(stream);
            while let Some(event) = stream.next().await {
                match event {
                    AgentEvent::TextDelta(text) => {
                        yield StreamEvent::Chunk { content: text };
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
                    _ => {}
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
        #[cfg(feature = "local")]
        {
            use tokio::sync::mpsc;
            async_stream::try_stream! {
                // Only registry models are supported.
                let entry = model::local::registry::find(&req.model)
                    .ok_or_else(|| anyhow::anyhow!(
                        "model '{}' is not in the registry", req.model
                    ))?;

                if !entry.fits() {
                    let required = entry.memory_requirement();
                    let actual = model::local::system_memory() / (1024 * 1024 * 1024);
                    Err(anyhow::anyhow!(
                        "model '{}' requires at least {} RAM, your system has {}GB",
                        entry.name, required, actual
                    ))?;
                }

                yield DownloadEvent::Start { model: req.model.clone() };

                let (dtx, mut drx) = mpsc::unbounded_channel();
                let model_str = req.model.to_string();
                let download_handle = tokio::spawn(async move {
                    model::local::download::download_model(&model_str, dtx).await
                });

                while let Some(event) = drx.recv().await {
                    let dl_event = match event {
                        model::local::download::DownloadEvent::FileStart { filename, size } => {
                            DownloadEvent::FileStart { model: req.model.clone(), filename, size }
                        }
                        model::local::download::DownloadEvent::Progress { bytes } => {
                            DownloadEvent::Progress { model: req.model.clone(), bytes }
                        }
                        model::local::download::DownloadEvent::FileEnd { filename } => {
                            DownloadEvent::FileEnd { model: req.model.clone(), filename }
                        }
                    };
                    yield dl_event;
                }

                match download_handle.await {
                    Ok(Ok(())) => {
                        yield DownloadEvent::End { model: req.model };
                    }
                    Ok(Err(e)) => {
                        Err(anyhow::anyhow!("download failed: {e}"))?;
                    }
                    Err(e) => {
                        Err(anyhow::anyhow!("download task panicked: {e}"))?;
                    }
                }
            }
        }
        #[cfg(not(feature = "local"))]
        {
            let _ = req;
            async_stream::stream! {
                yield Err(anyhow::anyhow!("this daemon was built without local model support"));
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

    fn hub(
        &self,
        package: CompactString,
        action: HubAction,
    ) -> impl futures_core::Stream<Item = Result<HubEvent>> + Send {
        async_stream::try_stream! {
            match action {
                HubAction::Install => {
                    let s = hub::install(package);
                    pin_mut!(s);
                    while let Some(event) = s.next().await {
                        yield event?;
                    }
                }
                HubAction::Uninstall => {
                    let s = hub::uninstall(package);
                    pin_mut!(s);
                    while let Some(event) = s.next().await {
                        yield event?;
                    }
                }
            }
        }
    }
}
