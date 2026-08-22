//! Standing event subscriptions, addressed by id.

use crate::{
    AgentId,
    kv::{Column, KVStorage},
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::future::Future;

/// A standing request to run an agent when a named event fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSubscription {
    pub id: u64,
    pub source: String,
    pub target_agent: AgentId,
    #[serde(default)]
    pub once: bool,
    /// The session recurring fires land in. Minted when the subscription
    /// is created — there is no client turn on a later fire to hand a
    /// fresh handle back to, so it has to already exist by then.
    #[serde(default)]
    pub session_handle: String,
}

impl From<&EventSubscription> for proto::SubscriptionInfo {
    fn from(sub: &EventSubscription) -> Self {
        Self {
            id: sub.id,
            source: sub.source.clone(),
            target_agent: sub.target_agent.to_string(),
            once: sub.once,
        }
    }
}

/// What the daemon has been asked to run on an event.
///
/// Subscriptions arrive through the protocol and are rewritten whenever
/// one is added, dropped, or fires `once`, so they are the daemon's own
/// state and belong beside the rest of it.
pub trait Subscriptions: Send + Sync + 'static {
    fn subscriptions(&self) -> impl Future<Output = Result<Vec<EventSubscription>>> + Send;

    fn put_subscription(&self, sub: &EventSubscription) -> impl Future<Output = Result<()>> + Send;

    /// Remove a subscription. `true` if it was there.
    fn remove_subscription(&self, id: u64) -> impl Future<Output = Result<bool>> + Send;
}

impl<T: KVStorage> Subscriptions for T {
    async fn subscriptions(&self) -> Result<Vec<EventSubscription>> {
        let mut out = Vec::new();
        for key in self
            .scan_keys(Column::Event, &self.prefix(&["subscription"]))
            .await?
        {
            if let Some(sub) = self.get_json(Column::Event, &key).await? {
                out.push(sub);
            }
        }
        Ok(out)
    }

    async fn put_subscription(&self, sub: &EventSubscription) -> Result<()> {
        self.put_json(
            Column::Event,
            &self.key(&["subscription", &sub.id.to_string()]),
            sub,
        )
        .await
    }

    async fn remove_subscription(&self, id: u64) -> Result<bool> {
        self.delete(Column::Event, &self.key(&["subscription", &id.to_string()]))
            .await
    }
}
