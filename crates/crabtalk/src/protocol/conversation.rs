//! Conversation operations: send/stream, kill, and ask/tool reply
//! routing. Pure-runtime ops live on `Runtime<C>` directly.

use crate::llm::Provider;
use crate::system::CrabTalk;
use anyhow::Result;
use futures_util::{StreamExt, pin_mut};
use proto::*;
use runtime::sessions::SearchOptions;
use schema::{AgentEvent, storage::Storage};
use std::sync::Arc;

/// Roles travel raw — a label like `tool:read` is presentation, and the
/// protocol carries `role` and `tool_name` separately so whatever renders
/// them can decide.
fn role_name(role: schema::model::Role) -> String {
    use schema::model::Role;
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
        Role::System => "system",
        _ => "other",
    }
    .to_owned()
}

impl<P: Provider + 'static, S: Storage> CrabTalk<P, S> {
    /// Ranked excerpts from past conversations.
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
            agent_filter: (!req.agent.is_empty()).then_some(req.agent),
            sender_filter: (!req.sender.is_empty()).then_some(req.sender),
        };

        Ok(rt
            .search_sessions(&req.query, &opts)
            .into_iter()
            .map(|hit| SessionHit {
                session_handle: hit.session_handle.as_str().to_owned(),
                msg_idx: hit.msg_idx,
                score: hit.score,
                title: hit.title,
                agent: hit.agent,
                sender: hit.sender,
                created_at: hit.created_at,
                updated_at: hit.updated_at,
                window: hit
                    .window
                    .into_iter()
                    .map(|item| SessionWindowItem {
                        role: role_name(item.role),
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
        let rt: Arc<_> = self.runtime.read().await.clone();
        let sender = req.sender.as_deref().unwrap_or("");
        let created_by = if sender.is_empty() { "user" } else { sender };
        let conversation_id = rt
            .get_or_create_conversation(&req.agent, created_by)
            .await?;
        let tool_choice = req
            .tool_choice
            .map(|s| schema::model::ToolChoice::from(s.as_str()));
        let client_tools = resolve_client_tools(&self.bridge, conversation_id, req.tools);
        let response = rt
            .send_to(
                conversation_id,
                &req.content,
                sender,
                tool_choice,
                client_tools,
            )
            .await?;
        let usage = Some(response.usage());
        Ok(SendResponse {
            agent: req.agent,
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
        let agent = req.agent;
        let content = req.content;
        let sender = req.sender.unwrap_or_default();
        let guest = req.guest.unwrap_or_default();
        let tool_choice = req
            .tool_choice
            .map(|s| schema::model::ToolChoice::from(s.as_str()));
        let req_tools = req.tools;
        let ephemeral = req.ephemeral;
        let correlation_id = req.correlation_id.unwrap_or(0);
        async_stream::try_stream! {
            let rt: Arc<_> = runtime.read().await.clone();

            // Anonymous, unpersisted turn: no conversation, no bridge
            // listener, no storage. Client round-trip tools are
            // unsupported here, so only the agent's own (daemon-side)
            // tools run.
            if ephemeral {
                yield StreamEvent { event: Some(stream_event::Event::Start(StreamStart { agent: agent.clone() })) };
                let stream = rt.ephemeral_stream(&agent, &content, correlation_id, tool_choice, vec![]);
                pin_mut!(stream);
                while let Some(event) = stream.next().await {
                    let is_done = matches!(event, AgentEvent::Done(_));
                    yield event.to_stream(&agent);
                    if is_done { return; }
                }
                yield StreamEvent { event: Some(stream_event::Event::End(StreamEnd {
                    agent: agent.clone(), error: String::new(), model: String::new(), usage: None,
                })) };
                return;
            }

            let created_by = if sender.is_empty() { "user".into() } else { sender.clone() };
            let conversation_id = rt.get_or_create_conversation(&agent, created_by.as_str()).await?;
            // Register this conversation as having a stream listener so the
            // bridge will forward dispatches here. The guard
            // unregisters on any exit path — stream end, early return on
            // Done, or consumer dropping the stream — and fails any
            // pending forwarded calls so they don't sit until timeout.
            bridge.register_listener(conversation_id);
            let _listener_guard = ListenerGuard::new(bridge.clone(), conversation_id);
            let client_tools = resolve_client_tools(&bridge, conversation_id, req_tools);

            let responding_agent = if guest.is_empty() { agent.clone() } else { guest.clone() };
            yield StreamEvent { event: Some(stream_event::Event::Start(StreamStart { agent: responding_agent.clone() })) };

            let stream: std::pin::Pin<Box<dyn futures_core::Stream<Item = schema::AgentEvent> + Send + '_>> = if guest.is_empty() {
                Box::pin(rt.stream_to(conversation_id, &content, &sender, tool_choice, client_tools))
            } else {
                Box::pin(rt.guest_stream_to(conversation_id, &content, &sender, &guest))
            };
            pin_mut!(stream);
            while let Some(event) = stream.next().await {
                // Client-tool forwards only exist on the persisted path,
                // where a bridge listener can carry the reply back.
                let forwards: Vec<ToolCallForwardEvent> = if let AgentEvent::ToolCallsStart(ref calls) = event {
                    calls
                        .iter()
                        .filter(|c| bridge.is_client_tool(conversation_id, &c.function.name))
                        .map(|c| ToolCallForwardEvent {
                            call_id: c.id.to_string(),
                            name: c.function.name.to_string(),
                            arguments: c.function.arguments.clone(),
                            conversation_id,
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
                agent: responding_agent,
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
        Ok(rt.close(conversation_id).await)
    }

    pub(crate) async fn reply_to_tool(
        &self,
        conversation_id: u64,
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
            .try_resolve(conversation_id, call_id, output, is_error)
        {
            Ok(())
        } else {
            anyhow::bail!("duplicate reply for call_id '{call_id}'")
        }
    }
}

/// RAII guard that synchronously unregisters a stream's client-tool
/// listener and drains pending forwarded calls on drop.
struct ListenerGuard {
    bridge: Arc<crate::bridge::ClientBridge>,
    conv_id: u64,
}

impl ListenerGuard {
    fn new(bridge: Arc<crate::bridge::ClientBridge>, conv_id: u64) -> Self {
        Self { bridge, conv_id }
    }
}

impl Drop for ListenerGuard {
    fn drop(&mut self) {
        self.bridge.unregister_listener(self.conv_id);
    }
}

fn resolve_client_tools(
    bridge: &crate::bridge::ClientBridge,
    conversation_id: u64,
    proto_tools: Vec<ToolDef>,
) -> Vec<crate::llm::Tool> {
    let tools: Vec<crate::llm::Tool> = proto_tools.into_iter().map(Into::into).collect();
    // Registered even when empty: a client that declares nothing has no
    // client tools, which is a different thing from having not been asked.
    bridge.register_tools(conversation_id, &tools);
    tools
}
