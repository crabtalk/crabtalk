//! Administrative operations: stats, events.

use crate::{
    llm::Provider,
    system::{CrabTalk, event::EventSubscription},
};
use anyhow::Result;
use proto::{
    AgentEventMsg, McpEventMsg, Stats, SubscribeEventMsg, SubscriptionInfo, SubscriptionList,
};
use runtime::Env;
use store::interface::Backend;
use tokio::sync::broadcast::error::RecvError;

impl<P: Provider + 'static, S: Backend> CrabTalk<P, S> {
    pub(crate) async fn get_stats(&self) -> Result<Stats> {
        let rt = self.runtime.read().await.clone();
        let active = self.sessions.count();
        let agents = rt.agents().await.len() as u32;
        let uptime = self.started_at.elapsed().as_secs();
        let active_model = rt.active_model().await;
        Ok(Stats {
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
                    Err(RecvError::Closed) => break,
                    Err(RecvError::Lagged(_)) => continue,
                }
            }
        }
    }

    pub(crate) fn subscribe_mcp_events(
        &self,
    ) -> impl futures_core::Stream<Item = Result<McpEventMsg>> + Send {
        let mcp = self.registry.mcp.handler.clone();
        async_stream::try_stream! {
            let mut rx = mcp.subscribe();
            loop {
                match rx.recv().await {
                    Ok(event) => yield event.into(),
                    Err(RecvError::Closed) => break,
                    Err(RecvError::Lagged(_)) => continue,
                }
            }
        }
    }

    pub(crate) async fn subscribe_event(&self, req: SubscribeEventMsg) -> Result<SubscriptionInfo> {
        let target = crate::protocol::parse_agent(&req.target_agent)?;
        let rt = self.runtime.read().await.clone();
        if rt.agent(&target).await.is_none() {
            anyhow::bail!("agent '{target}' not found");
        }
        let sub = EventSubscription {
            id: 0,
            source: req.source,
            target_agent: target,
            once: req.once,
            session_handle: ulid::Ulid::new().to_string(),
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
