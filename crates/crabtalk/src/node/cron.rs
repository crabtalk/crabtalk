//! Daemon-level cron scheduler.
//!
//! A cron entry triggers a skill into a session on a schedule.
//! Memory is authoritative at runtime; `cron/crons.toml` under the
//! config directory is recovery for restarts.

use crate::node::SharedRuntime;
use crabllm_core::Provider;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, str::FromStr, sync::Arc};
use tokio::{
    sync::{Mutex, broadcast},
    task::JoinHandle,
};

/// Persistent cron entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronEntry {
    pub id: u64,
    pub schedule: String,
    pub skill: String,
    pub agent: String,
    pub sender: String,
    #[serde(default)]
    pub quiet_start: Option<String>,
    #[serde(default)]
    pub quiet_end: Option<String>,
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
pub struct CronStore<P: Provider + 'static> {
    entries: HashMap<u64, CronEntry>,
    handles: HashMap<u64, JoinHandle<()>>,
    next_id: u64,
    path: PathBuf,
    runtime: SharedRuntime<P>,
    shutdown_tx: broadcast::Sender<()>,
}

fn validate_schedule(schedule: &str) -> Result<(), String> {
    cron::Schedule::from_str(schedule)
        .map(|_| ())
        .map_err(|e| format!("invalid cron schedule '{schedule}': {e}"))
}

impl<P: Provider + 'static> CronStore<P> {
    /// Load crons from `cron/crons.toml` under `root`.
    pub fn load(
        root: PathBuf,
        runtime: SharedRuntime<P>,
        shutdown_tx: broadcast::Sender<()>,
    ) -> Self {
        let path = root.join("cron").join("crons.toml");
        let mut entries = HashMap::new();
        let mut max_id = 0u64;
        if let Ok(content) = std::fs::read_to_string(&path) {
            match toml::from_str::<CronFile>(&content) {
                Ok(file) => {
                    for entry in file.cron {
                        if let Err(e) = validate_schedule(&entry.schedule) {
                            tracing::warn!("cron {}: {e}, skipping", entry.id);
                            continue;
                        }
                        max_id = max_id.max(entry.id);
                        entries.insert(entry.id, entry);
                    }
                }
                Err(e) => tracing::warn!("failed to parse {}: {e}", path.display()),
            }
        }
        Self {
            entries,
            handles: HashMap::new(),
            next_id: max_id + 1,
            path,
            runtime,
            shutdown_tx,
        }
    }

    pub fn start_all(&mut self, store: Arc<Mutex<CronStore<P>>>) {
        let ids: Vec<u64> = self.entries.keys().copied().collect();
        for id in ids {
            self.spawn_timer(id, store.clone());
        }
        if !self.entries.is_empty() {
            tracing::info!("started {} cron timer(s)", self.entries.len());
        }
    }

    pub fn create(
        &mut self,
        mut entry: CronEntry,
        store: Arc<Mutex<CronStore<P>>>,
    ) -> Result<CronEntry, String> {
        validate_schedule(&entry.schedule)?;
        entry.id = self.next_id;
        self.next_id += 1;
        self.entries.insert(entry.id, entry.clone());
        self.spawn_timer(entry.id, store);
        self.save();
        Ok(entry)
    }

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

    pub fn list(&self) -> Vec<CronEntry> {
        self.entries.values().cloned().collect()
    }

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
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&self.path, &content) {
            tracing::error!("failed to write {}: {e}", self.path.display());
        }
    }

    fn spawn_timer(&mut self, id: u64, store: Arc<Mutex<CronStore<P>>>) {
        let Some(entry) = self.entries.get(&id).cloned() else {
            return;
        };
        let runtime = self.runtime.clone();
        let shutdown_rx = self.shutdown_tx.subscribe();
        let once = entry.once;
        let handle = tokio::spawn(async move {
            run_cron_timer(entry, runtime, shutdown_rx).await;
            if once {
                tracing::info!("cron {id}: one-shot completed, removing");
                store.lock().await.delete(id);
            }
        });
        self.handles.insert(id, handle);
    }
}

async fn run_cron_timer<P: Provider + 'static>(
    entry: CronEntry,
    runtime: SharedRuntime<P>,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    let schedule = cron::Schedule::from_str(&entry.schedule).expect("pre-validated schedule");

    tracing::info!(
        "cron {}: started (schedule='{}', skill='{}', agent='{}', sender='{}', once={})",
        entry.id,
        entry.schedule,
        entry.skill,
        entry.agent,
        entry.sender,
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
            "cron {}: firing skill '{}' into agent='{}' sender='{}'",
            entry.id,
            entry.skill,
            entry.agent,
            entry.sender,
        );

        let rt = runtime.read().await.clone();
        let conversation_id = match rt
            .get_or_create_conversation(&entry.agent, &entry.sender)
            .await
        {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!("cron {}: get_or_create_conversation: {e}", entry.id);
                if entry.once {
                    return;
                }
                continue;
            }
        };
        let content = format!("/{}", entry.skill);
        if let Err(e) = rt
            .send_to(conversation_id, &content, &entry.sender, None)
            .await
        {
            tracing::warn!("cron {}: send_to: {e}", entry.id);
        }

        if entry.once {
            return;
        }
    }
}

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
