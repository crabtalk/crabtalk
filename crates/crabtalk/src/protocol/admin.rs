//! Daemon-level administrative operations: stats, events.
//! `reload` is defined alongside the daemon builder (crates/crabtalk/src/daemon/builder.rs).

use crate::daemon::Daemon;
use crate::daemon::event::EventSubscription;
use anyhow::Result;
use crabllm_core::Provider;
use mcp::McpEvent;
use runtime::Env;
use wcore::protocol::message::*;

fn mcp_event_to_msg(event: McpEvent) -> McpEventMsg {
    let now = chrono::Utc::now().to_rfc3339();
    match event {
        McpEvent::Connecting { agent, name } => McpEventMsg {
            kind: McpEventKind::Connecting.into(),
            name,
            tools: Vec::new(),
            error: String::new(),
            timestamp: now,
            agent,
        },
        McpEvent::Connected { agent, name, tools } => McpEventMsg {
            kind: McpEventKind::Connected.into(),
            name,
            tools,
            error: String::new(),
            timestamp: now,
            agent,
        },
        McpEvent::Failed { agent, name, error } => McpEventMsg {
            kind: McpEventKind::Failed.into(),
            name,
            tools: Vec::new(),
            error,
            timestamp: now,
            agent,
        },
        McpEvent::Disconnected { agent, name } => McpEventMsg {
            kind: McpEventKind::Disconnected.into(),
            name,
            tools: Vec::new(),
            error: String::new(),
            timestamp: now,
            agent,
        },
    }
}

impl<P: Provider + 'static> Daemon<P> {
    pub(crate) async fn get_stats(&self) -> Result<DaemonStats> {
        let rt = self.runtime.read().await.clone();
        let active = rt.conversation_count().await;
        let agents = rt.agents().len() as u32;
        let uptime = self.started_at.elapsed().as_secs();
        let active_model = rt.active_model().await;
        Ok(DaemonStats {
            uptime_secs: uptime,
            active_conversations: active as u32,
            registered_agents: agents,
            active_model,
        })
    }

    pub(crate) fn subscribe_events(
        &self,
    ) -> impl futures_core::Stream<Item = Result<AgentEventMsg>> + Send {
        let runtime = self.runtime.clone();
        async_stream::try_stream! {
            let rt = runtime.read().await.clone();
            let Some(mut rx) = rt.env.subscribe_events() else {
                return;
            };
            loop {
                match rx.recv().await {
                    Ok(event) => yield event,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        }
    }

    pub(crate) fn subscribe_mcp_events(
        &self,
    ) -> impl futures_core::Stream<Item = Result<McpEventMsg>> + Send {
        let mcp = self.mcp.clone();
        async_stream::try_stream! {
            let mut rx = mcp.subscribe();
            loop {
                match rx.recv().await {
                    Ok(event) => yield mcp_event_to_msg(event),
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        }
    }

    pub(crate) async fn subscribe_event(&self, req: SubscribeEventMsg) -> Result<SubscriptionInfo> {
        let rt = self.runtime.read().await.clone();
        if rt.agent(&req.target_agent).is_none() {
            anyhow::bail!("agent '{}' not found", req.target_agent);
        }
        let sub = EventSubscription {
            id: 0,
            source: req.source,
            target_agent: req.target_agent,
            once: req.once,
        };
        let created = self.events.lock().subscribe(sub);
        Ok(SubscriptionInfo::from(&created))
    }

    pub(crate) fn unsubscribe_event(&self, id: u64) -> bool {
        self.events.lock().unsubscribe(id)
    }

    pub(crate) fn list_subscriptions(&self) -> SubscriptionList {
        let subs = self.events.lock().list();
        SubscriptionList {
            subscriptions: subs.iter().map(SubscriptionInfo::from).collect(),
        }
    }

    pub(crate) fn publish_event(&self, source: &str, payload: &str) {
        self.events.lock().publish(source, payload);
    }
}
