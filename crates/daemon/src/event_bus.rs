//! Daemon event bus — subscription-based routing to agents.
//!
//! Subscriptions match on an exact `source` string. When a matching
//! event is published, the bus invokes a user-supplied `fire` callback.
//! Persistence is direct filesystem I/O to `events/subscriptions.toml`.

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

/// Persistent event subscription.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSubscription {
    pub id: u64,
    pub source: String,
    pub target_agent: String,
    #[serde(default)]
    pub once: bool,
}

/// TOML file wrapper — `[[subscription]]` array of tables.
#[derive(Debug, Default, Serialize, Deserialize)]
struct EventFile {
    #[serde(default)]
    subscription: Vec<EventSubscription>,
}

/// Callback signature for firing a matched subscription.
pub type FireCallback = Arc<dyn Fn(&EventSubscription, &str) + Send + Sync>;

/// In-memory event bus with filesystem-backed recovery.
pub struct EventBus {
    subscriptions: HashMap<u64, EventSubscription>,
    next_id: u64,
    path: PathBuf,
    fire: FireCallback,
}

impl EventBus {
    /// Load subscriptions from `events/subscriptions.toml` under `root`.
    pub fn load(root: PathBuf, fire: FireCallback) -> Self {
        let path = root.join("events").join("subscriptions.toml");
        let mut subscriptions = HashMap::new();
        let mut max_id = 0u64;
        if let Ok(content) = std::fs::read_to_string(&path) {
            match toml::from_str::<EventFile>(&content) {
                Ok(file) => {
                    for sub in file.subscription {
                        max_id = max_id.max(sub.id);
                        subscriptions.insert(sub.id, sub);
                    }
                }
                Err(e) => tracing::warn!("failed to parse {}: {e}", path.display()),
            }
        }
        Self {
            subscriptions,
            next_id: max_id + 1,
            path,
            fire,
        }
    }

    /// Create a new subscription.
    pub fn subscribe(&mut self, mut sub: EventSubscription) -> EventSubscription {
        sub.id = self.next_id;
        self.next_id += 1;
        self.subscriptions.insert(sub.id, sub.clone());
        self.save();
        sub
    }

    /// Remove a subscription.
    pub fn unsubscribe(&mut self, id: u64) -> bool {
        if self.subscriptions.remove(&id).is_none() {
            return false;
        }
        self.save();
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
        if !to_remove.is_empty() {
            for id in &to_remove {
                self.subscriptions.remove(id);
            }
            self.save();
        }
    }

    fn save(&self) {
        let file = EventFile {
            subscription: self.subscriptions.values().cloned().collect(),
        };
        let content = match toml::to_string_pretty(&file) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("failed to serialize subscriptions: {e}");
                return;
            }
        };
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&self.path, &content) {
            tracing::error!("failed to write {}: {e}", self.path.display());
        }
    }
}
