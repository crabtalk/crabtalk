//! Cron scheduler for periodic agent tasks.
//!
//! Each cron job runs in an isolated session with no state carried
//! between runs (DD#6). The scheduler is decoupled from the runtime —
//! it produces events (fires jobs), and the Gateway wires them to
//! agent dispatch.

use crate::config::CronConfig;
use chrono::Utc;
use compact_str::CompactString;
use cron::Schedule;
use std::str::FromStr;
use tokio::{sync::broadcast, task::JoinHandle, time};

/// A parsed cron job ready for scheduling.
#[derive(Debug, Clone)]
pub struct CronJob {
    /// Job name.
    pub name: CompactString,
    /// Parsed cron schedule.
    pub schedule: Schedule,
    /// Target agent name.
    pub agent: CompactString,
    /// Message template to send.
    pub message: String,
}

impl CronJob {
    /// Parse a [`CronJob`] from configuration.
    pub fn from_config(config: &CronConfig) -> anyhow::Result<Self> {
        let schedule = Schedule::from_str(&config.schedule)
            .map_err(|e| anyhow::anyhow!("invalid cron expression '{}': {e}", config.schedule))?;
        Ok(Self {
            name: config.name.clone(),
            schedule,
            agent: config.agent.clone(),
            message: config.message.clone(),
        })
    }
}

/// Cron scheduler that fires jobs on their schedules.
pub struct CronScheduler {
    jobs: Vec<CronJob>,
}

impl CronScheduler {
    /// Create a scheduler from a list of cron jobs.
    pub fn new(jobs: Vec<CronJob>) -> Self {
        Self { jobs }
    }

    /// Parse all cron configs into a scheduler.
    pub fn from_configs(configs: &[CronConfig]) -> anyhow::Result<Self> {
        let jobs = configs
            .iter()
            .map(CronJob::from_config)
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self { jobs })
    }

    /// Start the scheduler. Calls `on_fire` for each job when it fires.
    ///
    /// Returns a [`JoinHandle`]. The scheduler stops when `shutdown` is
    /// received or the handle is aborted.
    ///
    /// Before sleeping, the scheduler identifies which jobs are due at the
    /// soonest upcoming time. After waking it fires exactly those jobs,
    /// avoiding the ambiguity of re-querying `upcoming()` post-sleep.
    pub fn start<F, Fut>(self, on_fire: F, mut shutdown: broadcast::Receiver<()>) -> JoinHandle<()>
    where
        F: Fn(CronJob) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        tokio::spawn(async move {
            if self.jobs.is_empty() {
                tracing::info!("cron scheduler started with no jobs");
                let _ = shutdown.recv().await;
                return;
            }

            tracing::info!("cron scheduler started with {} job(s)", self.jobs.len());
            loop {
                let now = Utc::now();
                let mut due_jobs: Vec<usize> = Vec::new();
                let mut soonest = None::<chrono::DateTime<Utc>>;

                for (i, job) in self.jobs.iter().enumerate() {
                    if let Some(next) = job.schedule.upcoming(Utc).next() {
                        match soonest {
                            None => {
                                soonest = Some(next);
                                due_jobs.clear();
                                due_jobs.push(i);
                            }
                            Some(s) if next < s => {
                                soonest = Some(next);
                                due_jobs.clear();
                                due_jobs.push(i);
                            }
                            Some(s) if (next - s).num_seconds().abs() <= 0 => {
                                due_jobs.push(i);
                            }
                            _ => {}
                        }
                    }
                }

                let Some(soonest_time) = soonest else {
                    tracing::warn!("no upcoming cron fires, scheduler stopping");
                    return;
                };

                let wait = (soonest_time - now).to_std().unwrap_or_default();
                tokio::select! {
                    _ = time::sleep(wait) => {
                        for &i in &due_jobs {
                            tracing::info!("cron firing job '{}'", self.jobs[i].name);
                            on_fire(self.jobs[i].clone()).await;
                        }
                    }
                    _ = shutdown.recv() => {
                        tracing::info!("cron scheduler shutting down");
                        return;
                    }
                }
            }
        })
    }

    /// Get the list of jobs.
    pub fn jobs(&self) -> &[CronJob] {
        &self.jobs
    }
}
