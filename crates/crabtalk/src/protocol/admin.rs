//! Administrative handlers: ping, reload, stats, crons, events, services.

use crate::node::Node;
use crate::node::{cron::CronEntry, event::EventSubscription};
use anyhow::Result;
use crabllm_core::Provider;
use runtime::host::Host;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use wcore::protocol::message::*;

pub(super) async fn ping() -> Result<()> {
    Ok(())
}

pub(super) async fn reload<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
) -> Result<()> {
    node.reload().await
}

pub(super) async fn get_stats<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
) -> Result<DaemonStats> {
    let rt = node.runtime.read().await.clone();
    let active = rt.conversation_count().await;
    let agents = rt.agents().len() as u32;
    let uptime = node.started_at.elapsed().as_secs();
    let active_model = super::config::load_config(node)
        .await
        .ok()
        .and_then(|c| c.system.crab.model)
        .unwrap_or_default();
    Ok(DaemonStats {
        uptime_secs: uptime,
        active_conversations: active as u32,
        registered_agents: agents,
        active_model,
    })
}

pub(super) async fn create_cron<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
    req: CreateCronMsg,
) -> Result<CronInfo> {
    let rt = node.runtime.read().await.clone();
    if rt.agent(&req.agent).is_none() {
        anyhow::bail!("agent '{}' not found", req.agent);
    }
    let entry = CronEntry {
        id: 0,
        schedule: req.schedule,
        skill: req.skill,
        agent: req.agent,
        sender: req.sender,
        quiet_start: req.quiet_start,
        quiet_end: req.quiet_end,
        once: req.once,
    };
    let created = node
        .crons
        .lock()
        .await
        .create(entry, node.crons.clone())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(cron_entry_to_info(&created))
}

pub(super) async fn delete_cron<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
    id: u64,
) -> Result<bool> {
    Ok(node.crons.lock().await.delete(id))
}

pub(super) async fn list_crons<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
) -> Result<CronList> {
    let entries = node.crons.lock().await.list();
    Ok(CronList {
        crons: entries.iter().map(cron_entry_to_info).collect(),
    })
}

pub(super) fn subscribe_events<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
) -> impl futures_core::Stream<Item = Result<AgentEventMsg>> + Send {
    let runtime = node.runtime.clone();
    async_stream::try_stream! {
        let rt = runtime.read().await.clone();
        let Some(mut rx) = rt.hook.host.subscribe_events() else {
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

pub(super) async fn subscribe_event<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
    req: SubscribeEventMsg,
) -> Result<SubscriptionInfo> {
    let rt = node.runtime.read().await.clone();
    if rt.agent(&req.target_agent).is_none() {
        anyhow::bail!("agent '{}' not found", req.target_agent);
    }
    let sub = EventSubscription {
        id: 0,
        source: req.source,
        target_agent: req.target_agent,
        once: req.once,
    };
    let created = node
        .events
        .lock()
        .expect("event bus lock poisoned")
        .subscribe(sub);
    Ok(subscription_to_info(&created))
}

pub(super) async fn unsubscribe_event<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
    id: u64,
) -> Result<bool> {
    Ok(node
        .events
        .lock()
        .expect("event bus lock poisoned")
        .unsubscribe(id))
}

pub(super) async fn list_subscriptions<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
) -> Result<SubscriptionList> {
    let subs = node.events.lock().expect("event bus lock poisoned").list();
    Ok(SubscriptionList {
        subscriptions: subs.iter().map(subscription_to_info).collect(),
    })
}

pub(super) async fn publish_event<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
    req: PublishEventMsg,
) -> Result<()> {
    node.events
        .lock()
        .expect("event bus lock poisoned")
        .publish(&req.source, &req.payload);
    Ok(())
}

pub(super) async fn start_service<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
    name: String,
    force: bool,
) -> Result<()> {
    let cmd = find_command_service(node, &name)?;
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

pub(super) async fn stop_service(name: String) -> Result<()> {
    let label = format!("ai.crabtalk.{name}");
    command::service::uninstall(&label)?;
    let _ = std::fs::remove_file(wcore::paths::service_port_file(&name));
    Ok(())
}

pub(super) async fn service_logs(name: String, lines: u32) -> Result<String> {
    let path = wcore::paths::service_log_path(&name);
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

fn find_command_service<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
    name: &str,
) -> Result<plugin::manifest::CommandConfig> {
    for (_, manifest) in super::plugin::scan_plugin_manifests(&node.config_dir) {
        if let Some(cmd) = manifest.commands.get(name) {
            return Ok(cmd.clone());
        }
    }
    anyhow::bail!("command service '{name}' not found in installed plugins")
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

fn cron_entry_to_info(e: &CronEntry) -> CronInfo {
    CronInfo {
        id: e.id,
        schedule: e.schedule.clone(),
        skill: e.skill.clone(),
        agent: e.agent.clone(),
        quiet_start: e.quiet_start.clone().unwrap_or_default(),
        quiet_end: e.quiet_end.clone().unwrap_or_default(),
        once: e.once,
        sender: e.sender.clone(),
    }
}

fn subscription_to_info(sub: &EventSubscription) -> SubscriptionInfo {
    SubscriptionInfo {
        id: sub.id,
        source: sub.source.clone(),
        target_agent: sub.target_agent.clone(),
        once: sub.once,
    }
}
