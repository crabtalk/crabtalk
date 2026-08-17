//! Session operations: send/stream, kill, and ask/tool reply
//! routing. Pure-runtime ops live on `Runtime<C>` directly.

use crate::llm::Provider;
use crate::protocol::parse_agent;
use crate::system::CrabTalk;
use anyhow::Result;
use futures_util::{StreamExt, pin_mut};
use proto::*;
use runtime::{AgentEvent, Sessions};
use std::sync::Arc;
use storage::{AgentId, SearchOptions, Storage};

impl<P: Provider + 'static, S: Storage> CrabTalk<P, S> {
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

    pub(crate) async fn send(&self, req: SendMsg) -> Result<SendResponse> {
        let agent = parse_agent(&req.agent)?;
        let rt: Arc<_> = self.runtime.read().await.clone();
        let sender = req.sender.as_deref().unwrap_or("");
        let created_by = if sender.is_empty() { "user" } else { sender };
        let (session_id, session) = self.sessions.get_or_create(&rt, &agent, created_by).await?;
        let tool_choice = req
            .tool_choice
            .map(|s| crabllm_core::ToolChoice::from(s.as_str()));
        let client_tools = resolve_client_tools(&self.bridge, session_id, req.tools);
        let response = rt
            .send_to(&session, &req.content, sender, tool_choice, client_tools)
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
        let bridge = self.bridge.clone();
        let sessions = self.sessions.clone();
        let agent = req.agent;
        let content = req.content;
        let sender = req.sender.unwrap_or_default();
        let guest = req.guest.unwrap_or_default();
        let tool_choice = req
            .tool_choice
            .map(|s| crabllm_core::ToolChoice::from(s.as_str()));
        let req_tools = req.tools;
        let ephemeral = req.ephemeral;
        let correlation_id = req.correlation_id.unwrap_or(0);
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

            let created_by = if sender.is_empty() { "user".into() } else { sender.clone() };
            let (session_id, session) =
                sessions.get_or_create(&rt, &agent, created_by.as_str()).await?;
            // Register this session as having a stream listener so the
            // bridge will forward dispatches here, and open its steering
            // channel. The guard tears both down on any exit path — stream
            // end, early return on Done, or consumer dropping the stream —
            // failing any pending forwarded calls so they don't sit until
            // timeout, and closing the steering channel so a later steer
            // reports "no active stream" instead of being swallowed.
            bridge.register_listener(session_id);
            let _listener_guard =
                ListenerGuard::new(bridge.clone(), sessions.clone(), session_id);
            let client_tools = resolve_client_tools(&bridge, session_id, req_tools);

            let responding_agent = guest.unwrap_or(agent);
            yield StreamEvent { event: Some(stream_event::Event::Start(StreamStart { agent: responding_agent.to_string() })) };

            let stream: std::pin::Pin<Box<dyn futures_core::Stream<Item = runtime::AgentEvent> + Send + '_>> = match guest {
                // Only this path reads a steer. Opening the channel for a
                // guest turn too would accept a steer nobody delivers.
                None => {
                    let steer = sessions.begin_stream(session_id);
                    Box::pin(rt.stream_to(session, &content, &sender, tool_choice, client_tools, steer))
                }
                Some(guest) => Box::pin(rt.guest_stream_to(session, &content, &sender, &guest)),
            };
            pin_mut!(stream);
            while let Some(event) = stream.next().await {
                // Client-tool forwards only exist on the persisted path,
                // where a bridge listener can carry the reply back.
                let forwards: Vec<ToolCallForwardEvent> = if let AgentEvent::ToolCallsStart(ref calls) = event {
                    calls
                        .iter()
                        .filter(|c| bridge.is_client_tool(session_id, &c.function.name))
                        .map(|c| ToolCallForwardEvent {
                            call_id: c.id.to_string(),
                            name: c.function.name.to_string(),
                            arguments: c.function.arguments.clone(),
                            // The wire still says "conversation".
                            conversation_id: session_id,
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                let is_done = matches!(event, AgentEvent::Done(_));
                yield event.to_stream(&responding_agent);
                for fwd in forwards {
                    yield StreamEvent { event: Some(stream_event::Event::ToolCallForward(fwd)) };
                }
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

    pub(crate) async fn kill_conversation(&self, agent: &AgentId, sender: &str) -> Result<bool> {
        let Some((session_id, _)) = self.sessions.find(agent, sender) else {
            return Ok(false);
        };
        // The session's client-tool state is keyed by the same id and
        // outlives it otherwise: a kill mid-stream would leave forwarded
        // calls parked until their 300s timeout.
        self.bridge.unregister_listener(session_id);
        Ok(self.sessions.close(session_id))
    }

    pub(crate) async fn reply_to_tool(
        &self,
        session_id: u64,
        call_id: &str,
        output: String,
        is_error: bool,
    ) -> Result<()> {
        // No retry needed: `try_resolve` accepts replies that arrive
        // before the agent's dispatch parks (stashed as `EarlyReply`),
        // so the dispatch/reply race is handled symmetrically inside
        // the bridge rather than via sleep-and-pray here.
        if self
            .bridge
            .try_resolve(session_id, call_id, output, is_error)
        {
            Ok(())
        } else {
            anyhow::bail!("duplicate reply for call_id '{call_id}'")
        }
    }
}

/// RAII guard that synchronously tears down everything keyed to a
/// stream: the client-tool listener and its pending forwarded calls, and
/// the session's steering channel.
struct ListenerGuard {
    bridge: Arc<crate::bridge::ClientBridge>,
    sessions: Arc<Sessions>,
    conv_id: u64,
}

impl ListenerGuard {
    fn new(
        bridge: Arc<crate::bridge::ClientBridge>,
        sessions: Arc<Sessions>,
        conv_id: u64,
    ) -> Self {
        Self {
            bridge,
            sessions,
            conv_id,
        }
    }
}

impl Drop for ListenerGuard {
    fn drop(&mut self) {
        self.bridge.unregister_listener(self.conv_id);
        self.sessions.end_stream(self.conv_id);
    }
}

fn resolve_client_tools(
    bridge: &crate::bridge::ClientBridge,
    session_id: u64,
    proto_tools: Vec<ToolDef>,
) -> Vec<crate::llm::Tool> {
    let tools: Vec<crate::llm::Tool> = proto_tools.into_iter().map(Into::into).collect();
    // Registered even when empty: a client that declares nothing has no
    // client tools, which is a different thing from having not been asked.
    bridge.register_tools(session_id, &tools);
    tools
}
