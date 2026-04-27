//! Daemon-level administrative operations: stats, events, services.
//! `reload` is defined alongside the daemon builder (crates/crabtalk/src/daemon/builder.rs).

use crate::daemon::Daemon;
use crate::daemon::event::EventSubscription;
use anyhow::Result;
use crabllm_core::Provider;
use runtime::Env;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use wcore::protocol::message::*;

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

    pub(crate) async fn start_service(&self, name: String, force: bool) -> Result<()> {
        let cmd = self.find_command_service(&name)?;
        let label = format!("ai.crabtalk.{name}");
        if !force && command::service::is_installed(&label) {
            anyhow::bail!("service '{name}' is already running, use force to restart");
        }
        let binary = find_binary(&cmd.krate)?;
        let rendered = command::service::render_service_template(
            &CommandService {
                name: name.clone(),
                description: cmd.description.clone(),
                label: label.clone(),
            },
            &binary,
        );
        command::service::install(&rendered, &label)
    }

    fn find_command_service(&self, name: &str) -> Result<plugin::manifest::CommandConfig> {
        for (_, manifest) in super::plugin::scan_plugin_manifests(&self.config_dir) {
            if let Some(cmd) = manifest.commands.get(name) {
                return Ok(cmd.clone());
            }
        }
        anyhow::bail!("command service '{name}' not found in installed plugins")
    }
}

pub(super) async fn stop_service(name: &str) -> Result<()> {
    let label = format!("ai.crabtalk.{name}");
    command::service::uninstall(&label)?;
    let _ = std::fs::remove_file(wcore::paths::service_port_file(name));
    Ok(())
}

pub(super) async fn service_logs(name: &str, lines: u32) -> Result<String> {
    let path = wcore::paths::service_log_path(name);
    if !path.exists() {
        return Ok(format!("no logs yet: {}", path.display()));
    }
    let file = std::fs::File::open(&path)
        .map_err(|e| anyhow::anyhow!("failed to open {}: {e}", path.display()))?;
    let n = if lines == 0 { 50 } else { lines as usize };
    let mut tail: VecDeque<String> = VecDeque::with_capacity(n);
    for line in BufReader::new(file).lines() {
        let line = line?;
        if tail.len() == n {
            tail.pop_front();
        }
        tail.push_back(line);
    }
    Ok(tail.into_iter().collect::<Vec<_>>().join("\n"))
}

fn find_binary(name: &str) -> Result<std::path::PathBuf> {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    let cargo_bin = wcore::paths::CONFIG_DIR
        .parent()
        .unwrap_or(std::path::Path::new("/"))
        .join(".cargo/bin")
        .join(name);
    if cargo_bin.exists() {
        return Ok(cargo_bin);
    }
    anyhow::bail!("binary '{name}' not found in PATH or ~/.cargo/bin")
}

struct CommandService {
    name: String,
    description: String,
    label: String,
}

impl command::service::Service for CommandService {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn label(&self) -> &str {
        &self.label
    }
}
