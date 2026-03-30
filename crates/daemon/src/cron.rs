//! Daemon-level cron scheduler.
//!
//! A cron entry triggers a skill into a session on a schedule. The session
//! carries the agent — no redundancy. Memory is authoritative at runtime;
//! disk (`crons.toml`) is recovery for restarts.

use crate::daemon::event::{DaemonEvent, DaemonEventSender};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, str::FromStr, sync::Arc};
use tokio::{
    sync::{Mutex, broadcast},
    task::JoinHandle,
};
use wcore::protocol::message::{ClientMessage, SendMsg};

/// Persistent cron entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronEntry {
    pub id: u64,
    pub schedule: String,
    pub skill: String,
    pub session: u64,
    #[serde(default)]
    pub quiet_start: Option<String>,
    #[serde(default)]
    pub quiet_end: Option<String>,
    /// Fire once then self-delete.
    #[serde(default)]
    pub once: bool,
}

/// TOML file wrapper — `[[cron]]` array of tables.
#[derive(Debug, Default, Serialize, Deserialize)]
struct CronFile {
    #[serde(default)]
    cron: Vec<CronEntry>,
}

/// In-memory cron store with per-entry timer tasks.
pub struct CronStore {
    entries: HashMap<u64, CronEntry>,
    handles: HashMap<u64, JoinHandle<()>>,
    next_id: u64,
    crons_path: PathBuf,
    event_tx: DaemonEventSender,
    shutdown_tx: broadcast::Sender<()>,
}

/// Validate that a cron schedule expression parses.
fn validate_schedule(schedule: &str) -> Result<(), String> {
    cron::Schedule::from_str(schedule)
        .map(|_| ())
        .map_err(|e| format!("invalid cron schedule '{schedule}': {e}"))
}

impl CronStore {
    /// Load crons from disk. Missing file is not an error.
    /// Entries with invalid schedules are skipped with a warning.
    pub fn load(
        crons_path: PathBuf,
        event_tx: DaemonEventSender,
        shutdown_tx: broadcast::Sender<()>,
    ) -> Self {
        let mut entries = HashMap::new();
        let mut max_id = 0u64;
        if let Ok(content) = std::fs::read_to_string(&crons_path) {
            if let Ok(file) = toml::from_str::<CronFile>(&content) {
                for entry in file.cron {
                    if let Err(e) = validate_schedule(&entry.schedule) {
                        tracing::warn!("cron {}: {e}, skipping", entry.id);
                        continue;
                    }
                    max_id = max_id.max(entry.id);
                    entries.insert(entry.id, entry);
                }
            } else {
                tracing::warn!(
                    "failed to parse {}, starting with empty crons",
                    crons_path.display()
                );
            }
        }
        Self {
            entries,
            handles: HashMap::new(),
            next_id: max_id + 1,
            crons_path,
            event_tx,
            shutdown_tx,
        }
    }

    /// Spawn timer tasks for all loaded entries.
    pub fn start_all(&mut self, store: Arc<Mutex<CronStore>>) {
        let ids: Vec<u64> = self.entries.keys().copied().collect();
        for id in ids {
            self.spawn_timer(id, store.clone());
        }
        if !self.entries.is_empty() {
            tracing::info!("started {} cron timer(s)", self.entries.len());
        }
    }

    /// Create a new cron entry. Validates the schedule, assigns an ID,
    /// spawns the timer, and persists to disk.
    pub fn create(
        &mut self,
        mut entry: CronEntry,
        store: Arc<Mutex<CronStore>>,
    ) -> Result<CronEntry, String> {
        validate_schedule(&entry.schedule)?;
        entry.id = self.next_id;
        self.next_id += 1;
        self.entries.insert(entry.id, entry.clone());
        self.spawn_timer(entry.id, store);
        self.save();
        Ok(entry)
    }

    /// Delete a cron entry. Aborts its timer and persists to disk.
    pub fn delete(&mut self, id: u64) -> bool {
        if self.entries.remove(&id).is_none() {
            return false;
        }
        if let Some(handle) = self.handles.remove(&id) {
            handle.abort();
        }
        self.save();
        true
    }

    /// List all cron entries.
    pub fn list(&self) -> Vec<CronEntry> {
        self.entries.values().cloned().collect()
    }

    /// Write-through to `crons.toml`. Uses atomic write (tmp + rename).
    fn save(&self) {
        let file = CronFile {
            cron: self.entries.values().cloned().collect(),
        };
        let content = match toml::to_string_pretty(&file) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("failed to serialize crons: {e}");
                return;
            }
        };
        let tmp = self.crons_path.with_extension("toml.tmp");
        if let Err(e) = std::fs::write(&tmp, &content) {
            tracing::error!("failed to write {}: {e}", tmp.display());
            return;
        }
        if let Err(e) = std::fs::rename(&tmp, &self.crons_path) {
            tracing::error!(
                "failed to rename {} -> {}: {e}",
                tmp.display(),
                self.crons_path.display()
            );
        }
    }

    /// Spawn a timer task for a single entry.
    /// One-shot crons self-delete through the `store` handle after firing.
    fn spawn_timer(&mut self, id: u64, store: Arc<Mutex<CronStore>>) {
        let Some(entry) = self.entries.get(&id).cloned() else {
            return;
        };
        let event_tx = self.event_tx.clone();
        let shutdown_rx = self.shutdown_tx.subscribe();
        let once = entry.once;
        let handle = tokio::spawn(async move {
            run_cron_timer(entry, event_tx, shutdown_rx).await;
            if once {
                tracing::info!("cron {id}: one-shot completed, removing");
                store.lock().await.delete(id);
            }
        });
        self.handles.insert(id, handle);
    }
}

/// Run a single cron timer loop until shutdown.
/// Returns after first fire when `entry.once` is true.
async fn run_cron_timer(
    entry: CronEntry,
    event_tx: DaemonEventSender,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    // Schedule was validated on create/load — unwrap is safe.
    let schedule = cron::Schedule::from_str(&entry.schedule).expect("pre-validated schedule");

    tracing::info!(
        "cron {}: started (schedule='{}', skill='{}', session={}, once={})",
        entry.id,
        entry.schedule,
        entry.skill,
        entry.session,
        entry.once,
    );

    loop {
        let next = match schedule.upcoming(chrono::Utc).next() {
            Some(t) => t,
            None => {
                tracing::warn!("cron {}: no upcoming fire times", entry.id);
                return;
            }
        };
        let until = (next - chrono::Utc::now())
            .to_std()
            .unwrap_or(std::time::Duration::ZERO);

        tokio::select! {
            _ = tokio::time::sleep(until) => {}
            _ = shutdown_rx.recv() => {
                tracing::debug!("cron {}: shutting down", entry.id);
                return;
            }
        }

        if is_quiet(entry.quiet_start.as_deref(), entry.quiet_end.as_deref()) {
            tracing::debug!("cron {}: skipped (quiet hours)", entry.id);
            continue;
        }

        tracing::info!(
            "cron {}: firing skill '{}' into session {}",
            entry.id,
            entry.skill,
            entry.session
        );

        // Fire-and-forget: receiver is dropped, so the first send() in
        // handle_message returns Err and the loop exits. Output goes to
        // session history only.
        let (reply_tx, _) = tokio::sync::mpsc::channel(1);
        let msg = ClientMessage::from(SendMsg {
            agent: String::new(),
            content: format!("/{}", entry.skill),
            session: Some(entry.session),
            sender: Some("cron".to_string()),
            cwd: None,
            new_chat: false,
            resume_file: None,
        });
        let _ = event_tx.send(DaemonEvent::Message {
            msg,
            reply: reply_tx,
        });

        if entry.once {
            return;
        }
    }
}

/// Check if the current local time is inside a quiet window.
fn is_quiet(quiet_start: Option<&str>, quiet_end: Option<&str>) -> bool {
    let (Some(qs), Some(qe)) = (quiet_start, quiet_end) else {
        return false;
    };
    let Ok(qs) = chrono::NaiveTime::parse_from_str(qs, "%H:%M") else {
        return false;
    };
    let Ok(qe) = chrono::NaiveTime::parse_from_str(qe, "%H:%M") else {
        return false;
    };
    let now = chrono::Local::now().time();
    if qs <= qe {
        now >= qs && now < qe
    } else {
        now >= qs || now < qe
    }
}
