//! Daemon event bus — subscription-based routing to agents.
//!
//! Subscriptions match on an exact `source` string. When a matching
//! event is published, the bus invokes a user-supplied `fire` callback.
//! The map here is what this process answers from; the store is its
//! durable mirror.

use std::{collections::HashMap, sync::Arc};
use store::{EventSubscription, Subscriptions};

/// Callback signature for firing a matched subscription.
pub type FireCallback = Arc<dyn Fn(&EventSubscription, &str) + Send + Sync>;

/// In-memory event bus, recovered from and mirrored to the store.
pub struct EventBus<S: Subscriptions> {
    subscriptions: HashMap<u64, EventSubscription>,
    next_id: u64,
    storage: Arc<S>,
    fire: FireCallback,
}

impl<S: Subscriptions> EventBus<S> {
    /// Recover what was subscribed before this process started.
    pub async fn load(storage: Arc<S>, fire: FireCallback) -> Self {
        let subscriptions: HashMap<u64, EventSubscription> = match storage.subscriptions().await {
            Ok(subs) => subs.into_iter().map(|sub| (sub.id, sub)).collect(),
            Err(e) => {
                tracing::warn!("failed to read subscriptions: {e}");
                HashMap::new()
            }
        };
        let next_id = subscriptions.keys().max().copied().unwrap_or(0) + 1;
        Self {
            subscriptions,
            next_id,
            storage,
            fire,
        }
    }

    /// Create a new subscription.
    pub fn subscribe(&mut self, mut sub: EventSubscription) -> EventSubscription {
        sub.id = self.next_id;
        self.next_id += 1;
        self.subscriptions.insert(sub.id, sub.clone());
        self.mirror(Some(sub.clone()), sub.id);
        sub
    }

    /// Remove a subscription.
    pub fn unsubscribe(&mut self, id: u64) -> bool {
        if self.subscriptions.remove(&id).is_none() {
            return false;
        }
        self.mirror(None, id);
        true
    }

    /// List all subscriptions.
    pub fn list(&self) -> Vec<EventSubscription> {
        self.subscriptions.values().cloned().collect()
    }

    /// Publish an event. Fires every matching subscription.
    pub fn publish(&mut self, source: &str, payload: &str) {
        let mut to_remove = Vec::new();
        for (id, sub) in &self.subscriptions {
            if sub.source == source {
                (self.fire)(sub, payload);
                if sub.once {
                    to_remove.push(*id);
                }
            }
        }
        for id in to_remove {
            self.subscriptions.remove(&id);
            self.mirror(None, id);
        }
    }

    /// Write one subscription through to the store, or erase it when
    /// `sub` is `None`.
    ///
    /// Off the caller's thread, because `publish` is reached from a sync
    /// sink and a protocol reply should not wait on a disk write. A
    /// failure costs durability, not the subscription this process is
    /// already serving, so it is logged like the file write it replaced.
    fn mirror(&self, sub: Option<EventSubscription>, id: u64) {
        let storage = self.storage.clone();
        tokio::spawn(async move {
            let result = match &sub {
                Some(sub) => storage.put_subscription(sub).await.map(|_| true),
                None => storage.remove_subscription(id).await,
            };
            if let Err(e) = result {
                tracing::error!("failed to persist subscription {id}: {e}");
            }
        });
    }
}
