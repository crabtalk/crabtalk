//! Daemon-level event bus — subscription-based routing.
//!
//! Subscriptions match on exact `source` strings and fire target agents
//! with the event payload as message content. Memory is authoritative at
//! runtime; disk (`events.toml`) is recovery for restarts.

use crate::daemon::event::{DaemonEvent, DaemonEventSender};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};
use wcore::protocol::message::{ClientMessage, SendMsg};

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

/// In-memory event bus with subscription routing.
pub struct EventBus {
    subscriptions: HashMap<u64, EventSubscription>,
    next_id: u64,
    events_path: PathBuf,
    event_tx: DaemonEventSender,
}

impl EventBus {
    /// Load subscriptions from disk. Missing file is not an error.
    pub fn load(events_path: PathBuf, event_tx: DaemonEventSender) -> Self {
        let mut subscriptions = HashMap::new();
        let mut max_id = 0u64;
        if let Ok(content) = std::fs::read_to_string(&events_path) {
            if let Ok(file) = toml::from_str::<EventFile>(&content) {
                for sub in file.subscription {
                    max_id = max_id.max(sub.id);
                    subscriptions.insert(sub.id, sub);
                }
            } else {
                tracing::warn!(
                    "failed to parse {}, starting with empty subscriptions",
                    events_path.display()
                );
            }
        }
        Self {
            subscriptions,
            next_id: max_id + 1,
            events_path,
            event_tx,
        }
    }

    /// Create a new subscription. Assigns an ID and persists to disk.
    pub fn subscribe(&mut self, mut sub: EventSubscription) -> EventSubscription {
        sub.id = self.next_id;
        self.next_id += 1;
        self.subscriptions.insert(sub.id, sub.clone());
        self.save();
        sub
    }

    /// Remove a subscription. Returns whether it existed.
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

    /// Publish an event. Fires all subscriptions matching `source` (exact match),
    /// removes `once` subscriptions after firing.
    pub fn publish(&mut self, source: &str, payload: &str) {
        let mut to_remove = Vec::new();
        for (id, sub) in &self.subscriptions {
            if sub.source == source {
                self.fire_agent(sub, payload);
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

    /// Fire a target agent with the event payload. Fire-and-forget.
    fn fire_agent(&self, sub: &EventSubscription, payload: &str) {
        tracing::info!(
            "event bus: firing agent='{}' for source='{}'",
            sub.target_agent,
            sub.source,
        );
        let (reply_tx, _) = tokio::sync::mpsc::channel(1);
        let msg = ClientMessage::from(SendMsg {
            agent: sub.target_agent.clone(),
            content: payload.to_owned(),
            sender: Some(format!("event:{}", sub.source)),
            cwd: None,
            guest: None,
        });
        let _ = self.event_tx.send(DaemonEvent::Message {
            msg,
            reply: reply_tx,
        });
    }

    /// Write-through to `events.toml`. Uses atomic write (tmp + rename).
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
        let tmp = self.events_path.with_extension("toml.tmp");
        if let Err(e) = std::fs::write(&tmp, &content) {
            tracing::error!("failed to write {}: {e}", tmp.display());
            return;
        }
        if let Err(e) = std::fs::rename(&tmp, &self.events_path) {
            tracing::error!(
                "failed to rename {} -> {}: {e}",
                tmp.display(),
                self.events_path.display()
            );
        }
    }
}
