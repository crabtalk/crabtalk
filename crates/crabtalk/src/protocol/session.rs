//! Session operations: send/stream, kill, and ask/tool reply
//! routing. Pure-runtime ops live on `Runtime<C>` directly.

use crate::{
    llm::Provider,
    protocol::{CancelGuard, parse_agent},
    system::CrabTalk,
};
use anyhow::{Context as _, Result};
use futures_util::{StreamExt, pin_mut};
use proto::*;
use runtime::AgentEvent;
use std::{path::PathBuf, sync::Arc};
use store::{SearchOptions, SessionHandle, interface::Backend};

impl<P: Provider + 'static, S: Backend> CrabTalk<P, S> {
    /// Ranked excerpts from past sessions.
    ///
    /// The index is the runtime's; this converts the request into its
    /// options and its hits back onto the wire. Defaults live in
    /// [`SearchOptions`], so an absent field means whatever the index
    /// considers sensible rather than a number repeated here.
    pub(crate) async fn search_sessions(&self, req: SearchSessionsMsg) -> Result<Vec<SessionHit>> {
        let rt = self.runtime.read().await.clone();
        let defaults = SearchOptions::default();
        let opts = SearchOptions {
            limit: req.limit.map_or(defaults.limit, |n| n as usize),
            context_before: req
                .context_before
                .map_or(defaults.context_before, |n| n as usize),
            context_after: req
                .context_after
                .map_or(defaults.context_after, |n| n as usize),
            agent_filter: match req.agent.as_str() {
                "" => None,
                raw => Some(parse_agent(raw)?),
            },
            sender_filter: (!req.sender.is_empty()).then_some(req.sender),
        };

        Ok(rt
            .storage()
            .search_sessions(&req.query, &opts)
            .await?
            .into_iter()
            .map(|hit| SessionHit {
                session_handle: hit.session_handle.as_str().to_owned(),
                msg_idx: hit.msg_idx,
                score: hit.score,
                title: hit.title,
                agent_id: hit.agent.to_string(),
                agent_name: hit.agent_name,
                sender: hit.sender,
                created_at: hit.created_at,
                updated_at: hit.updated_at,
                window: hit
                    .window
                    .into_iter()
                    .map(|item| SessionWindowItem {
                        role: item.role.as_str().to_owned(),
                        msg_idx: item.msg_idx,
                        snippet: item.snippet,
                        truncated: item.truncated,
                        tool_name: item.tool_name.unwrap_or_default(),
                    })
                    .collect(),
            })
            .collect())
    }

    /// Open a session under the handle the caller picked, bound to a root.
    ///
    /// Refuses a handle that already names a session: a client asking for a
    /// fresh one and silently receiving somebody else's history — under a
    /// root that is no longer the one it asked for — is the failure this
    /// exists to make loud.
    pub(crate) async fn create_session(&self, req: CreateSessionMsg) -> Result<()> {
        if req.session_handle.is_empty() {
            anyhow::bail!("session_handle is required");
        }
        let agent = parse_agent(&req.agent)?;
        let rt: Arc<_> = self.runtime.read().await.clone();
        let handle = SessionHandle::new(req.session_handle);
        if rt.storage().load_session(&handle).await?.is_some() {
            anyhow::bail!("session '{}' already exists", handle.as_str());
        }
        // Rejected here rather than left to the first tool call, which would
        // report a tool that is merely absent and say nothing about the root
        // that made it so.
        let root = req.root.map(PathBuf::from);
        let config = rt
            .agent(&agent)
            .await
            .ok_or_else(|| anyhow::anyhow!("agent '{agent}' not registered"))?;
        for declaration in &config.harnesses {
            crabtalk_berm::bind(declaration.root.as_ref(), root.as_deref()).with_context(|| {
                format!(
                    "harness '{}' cannot be bound to that root",
                    declaration.name
                )
            })?;
        }

        let sender = req.sender.as_deref().unwrap_or("");
        let created_by = if sender.is_empty() { "user" } else { sender };
        self.sessions
            .open(&rt, handle, &agent, created_by, root)
            .await?;
        Ok(())
    }

    pub(crate) async fn send(&self, req: SendMsg) -> Result<SendResponse> {
        if req.session_handle.is_empty() {
            anyhow::bail!("session_handle is required");
        }
        let agent = parse_agent(&req.agent)?;
        let rt: Arc<_> = self.runtime.read().await.clone();
        let sender = req.sender.as_deref().unwrap_or("");
        let created_by = if sender.is_empty() { "user" } else { sender };
        let handle = SessionHandle::new(req.session_handle);
        let (_, session) = self
            .sessions
            .open(&rt, handle, &agent, created_by, None)
            .await?;
        let tool_choice = req
            .tool_choice
            .map(|s| crabllm_core::ToolChoice::from(s.as_str()));
        let response = rt
            .send_to(&session, &req.content, sender, tool_choice, vec![])
            .await?;
        let usage = Some(response.usage());
        Ok(SendResponse {
            agent: agent.to_string(),
            content: response.final_response.unwrap_or_default(),
            model: response.model,
            usage,
        })
    }

    pub(crate) fn stream<'a>(
        &'a self,
        req: StreamMsg,
    ) -> impl futures_core::Stream<Item = Result<StreamEvent>> + Send + 'a {
        let runtime = self.runtime.clone();
        let sessions = self.sessions.clone();
        let agent = req.agent;
        let content = req.content;
        let sender = req.sender.unwrap_or_default();
        let guest = req.guest.unwrap_or_default();
        let tool_choice = req
            .tool_choice
            .map(|s| crabllm_core::ToolChoice::from(s.as_str()));
        let ephemeral = req.ephemeral;
        let correlation_id = req.correlation_id.unwrap_or(0);
        let session_handle = req.session_handle;
        async_stream::try_stream! {
            let agent = parse_agent(&agent)?;
            let guest = match guest.as_str() {
                "" => None,
                raw => Some(parse_agent(raw)?),
            };
            let rt: Arc<_> = runtime.read().await.clone();

            // Anonymous, unpersisted turn: no session, no bridge
            // listener, no storage. Client round-trip tools are
            // unsupported here, so only the agent's own (daemon-side)
            // tools run.
            if ephemeral {
                yield StreamEvent { event: Some(stream_event::Event::Start(StreamStart { agent: agent.to_string() })) };
                let stream = rt.ephemeral_stream(&agent, &content, correlation_id, tool_choice, vec![]);
                pin_mut!(stream);
                while let Some(event) = stream.next().await {
                    let is_done = matches!(event, AgentEvent::Done(_));
                    yield event.to_stream(&agent);
                    if is_done { return; }
                }
                yield StreamEvent { event: Some(stream_event::Event::End(StreamEnd {
                    agent: agent.to_string(), error: String::new(), model: String::new(), usage: None,
                })) };
                return;
            }

            if session_handle.is_empty() {
                Err(anyhow::anyhow!("session_handle is required"))?;
            }
            let created_by = if sender.is_empty() { "user".into() } else { sender.clone() };
            let (session_id, session) = sessions
                .open(&rt, SessionHandle::new(session_handle), &agent, created_by.as_str(), None)
                .await?;
            // Clears the cancellation token on any exit path — stream end,
            // early return on Done, or the consumer dropping the stream —
            // so a later cancel reports "no cancellable operation" instead of
            // being swallowed.
            let _cancel_guard = CancelGuard::new(sessions.clone(), session_id);

            let responding_agent = guest.unwrap_or(agent);
            yield StreamEvent { event: Some(stream_event::Event::Start(StreamStart { agent: responding_agent.to_string() })) };

            let stream: std::pin::Pin<Box<dyn futures_core::Stream<Item = runtime::AgentEvent> + Send + '_>> = match guest {
                // Only this path is cancellable. Opening the token for a
                // guest turn too would accept a cancel nobody delivers.
                None => {
                    let cancel = sessions.begin_cancel(session_id);
                    Box::pin(rt.stream_to(session, &content, &sender, tool_choice, vec![], cancel))
                }
                Some(guest) => Box::pin(rt.guest_stream_to(session, &content, &sender, &guest)),
            };
            pin_mut!(stream);
            while let Some(event) = stream.next().await {
                let is_done = matches!(event, AgentEvent::Done(_));
                yield event.to_stream(&responding_agent);
                if is_done { return; }
            }
            yield StreamEvent { event: Some(stream_event::Event::End(StreamEnd {
                agent: responding_agent.to_string(),
                error: String::new(),
                model: String::new(),
                usage: None,
            })) };
        }
    }

    pub(crate) async fn kill_conversation(&self, session_handle: &str) -> Result<bool> {
        let handle = SessionHandle::new(session_handle);
        let Some((session_id, _)) = self.sessions.find_by_handle(&handle) else {
            return Ok(false);
        };
        Ok(self.sessions.close(session_id))
    }
}
