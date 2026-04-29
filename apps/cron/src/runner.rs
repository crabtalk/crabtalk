//! Scheduler loop — spawns one timer task per schedule, polls the store
//! file for external edits, and fires `/{skill}` into the daemon on cue.

use crate::entry::{CronEntry, is_quiet};
use crate::store::Store;
use anyhow::Result;
use sdk::ConnectionInfo;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::{
    sync::{Mutex, broadcast},
    task::JoinHandle,
};
use wcore::protocol::message::{ClientMessage, StreamMsg};

const FILE_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Run the scheduler until a shutdown signal is received.
///
/// `schedule_path` is the TOML file owned by this process. `conn_info` is the
/// daemon connection info — constructed by the caller so tests and alternate
/// assemblies can inject their own.
pub async fn run(schedule_path: PathBuf, conn_info: ConnectionInfo) -> Result<()> {
    let store = Arc::new(Mutex::new(Store::load(schedule_path.clone())?));
    let (shutdown_tx, _) = broadcast::channel::<()>(1);

    let mut timers: HashMap<u64, JoinHandle<()>> = HashMap::new();
    reconcile(&store, &conn_info, &shutdown_tx, &mut timers).await;
    tracing::info!(
        "cron started — {} schedule(s) loaded from {}",
        timers.len(),
        schedule_path.display(),
    );

    let mut last_mtime = mtime(&schedule_path);
    let mut poll = tokio::time::interval(FILE_POLL_INTERVAL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let shutdown = install_shutdown_handler();

    loop {
        tokio::select! {
            _ = poll.tick() => {
                let current = mtime(&schedule_path);
                if current != last_mtime {
                    last_mtime = current;
                    tracing::debug!("schedule file changed, reloading");
                    match Store::load(schedule_path.clone()) {
                        Ok(fresh) => {
                            *store.lock().await = fresh;
                            reconcile(&store, &conn_info, &shutdown_tx, &mut timers).await;
                        }
                        Err(e) => tracing::warn!("reload failed: {e}"),
                    }
                }
            }
            _ = shutdown.notified() => {
                tracing::info!("cron shutting down");
                let _ = shutdown_tx.send(());
                break;
            }
        }
    }

    for (_, handle) in timers.drain() {
        handle.abort();
    }
    Ok(())
}

/// Start timers for entries that don't have one, abort timers whose entry
/// is gone. Assumes the store holds the fresh state.
async fn reconcile(
    store: &Arc<Mutex<Store>>,
    conn_info: &ConnectionInfo,
    shutdown_tx: &broadcast::Sender<()>,
    timers: &mut HashMap<u64, JoinHandle<()>>,
) {
    let active: Vec<CronEntry> = store.lock().await.list().to_vec();
    let active_ids: HashSet<u64> = active.iter().map(|e| e.id).collect();

    timers.retain(|id, handle| {
        if !active_ids.contains(id) {
            handle.abort();
            false
        } else {
            true
        }
    });

    for entry in active {
        if timers.contains_key(&entry.id) {
            continue;
        }
        let handle = spawn_timer(
            entry.clone(),
            conn_info.clone(),
            store.clone(),
            shutdown_tx.subscribe(),
        );
        timers.insert(entry.id, handle);
    }
}

fn spawn_timer(
    entry: CronEntry,
    conn_info: ConnectionInfo,
    store: Arc<Mutex<Store>>,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> JoinHandle<()> {
    let schedule = match ::cron::Schedule::from_str(&entry.schedule) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                "cron {}: invalid schedule '{}': {e}",
                entry.id,
                entry.schedule
            );
            return tokio::spawn(async {});
        }
    };

    tokio::spawn(async move {
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
            let Some(next) = schedule.upcoming(chrono::Utc).next() else {
                tracing::warn!("cron {}: no upcoming fire times", entry.id);
                return;
            };
            let until = (next - chrono::Utc::now())
                .to_std()
                .unwrap_or(Duration::ZERO);

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

            fire(&conn_info, &entry).await;

            if entry.once {
                if let Err(e) = store.lock().await.delete(entry.id) {
                    tracing::warn!("cron {}: delete after once-fire failed: {e}", entry.id);
                }
                tracing::info!("cron {}: one-shot completed", entry.id);
                return;
            }
        }
    })
}

/// Open a connection, fire a single StreamMsg, and drain the reply stream.
/// Errors inside the daemon surface as ErrorMsg in the stream and are logged
/// by the SDK — the schedule continues on the next tick regardless.
async fn fire(conn_info: &ConnectionInfo, entry: &CronEntry) {
    let msg = ClientMessage::from(StreamMsg {
        agent: entry.agent.clone(),
        content: format!("/{}", entry.skill),
        sender: Some(entry.sender.clone()),
        cwd: None,
        guest: None,
        tool_choice: None,
    });
    let mut rx = match conn_info.send(msg).await {
        Ok(rx) => rx,
        Err(e) => {
            tracing::warn!("cron {}: failed to connect to daemon: {e}", entry.id);
            return;
        }
    };
    while rx.recv().await.is_some() {}
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Listen for SIGINT / SIGTERM and signal shutdown once.
fn install_shutdown_handler() -> Arc<tokio::sync::Notify> {
    let notify = Arc::new(tokio::sync::Notify::new());
    let notify_ret = notify.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            let mut sigint = signal(SignalKind::interrupt()).expect("sigint handler");
            let mut sigterm = signal(SignalKind::terminate()).expect("sigterm handler");
            tokio::select! {
                _ = sigint.recv() => {}
                _ = sigterm.recv() => {}
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        notify.notify_waiters();
    });
    notify_ret
}
