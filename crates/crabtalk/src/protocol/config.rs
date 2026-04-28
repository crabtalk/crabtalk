//! Daemon-level configuration mutations: active model, MCP, skills.

use crate::daemon::Daemon;
use anyhow::{Context, Result};
use crabllm_core::Provider;
use mcp::{McpServerState, ServerStatus};
use std::collections::BTreeMap;
use wcore::protocol::message::*;
use wcore::storage::Storage;

impl<P: Provider + 'static> Daemon<P> {
    pub(crate) async fn set_active_model(&self, model: String) -> Result<()> {
        let rt = self.runtime.read().await.clone();
        let storage = rt.storage();

        // Validate against the cached model list when non-empty; if the
        // /v1/models fetch at startup failed, trust the caller.
        let known = rt.list_models().await;
        if !known.is_empty() && !known.iter().any(|m| m.name == model) {
            anyhow::bail!("model '{model}' not advertised by the LLM endpoint");
        }

        let mut crab = storage
            .load_agent_by_name(wcore::paths::DEFAULT_AGENT)
            .await?
            .unwrap_or_else(|| crate::storage::default_crab(&model));
        let prompt = std::mem::take(&mut crab.system_prompt);
        crab.model = model;
        storage.upsert_agent(&crab, &prompt).await?;
        self.reload().await
    }

    pub(crate) async fn list_mcps(&self, agent: Option<String>) -> Result<Vec<McpInfo>> {
        let states = self.mcp.states();
        let rt = self.runtime.read().await.clone();
        let mut by_name: BTreeMap<String, McpInfo> = BTreeMap::new();
        match agent {
            Some(name) => {
                let cfg = rt
                    .agent(&name)
                    .ok_or_else(|| anyhow::anyhow!("agent '{name}' not found"))?;
                for mcp_cfg in &cfg.mcps {
                    by_name.insert(mcp_cfg.name.clone(), mcp_info(mcp_cfg, &name, &states));
                }
            }
            None => {
                // Union view across every registered agent. First-declarer
                // wins on name conflicts (Phase 2 — fingerprint-keyed dedup
                // arrives in Phase 3).
                for cfg in rt.agents() {
                    for mcp_cfg in &cfg.mcps {
                        by_name
                            .entry(mcp_cfg.name.clone())
                            .or_insert_with(|| mcp_info(mcp_cfg, &cfg.name, &states));
                    }
                }
            }
        }
        Ok(by_name.into_values().collect())
    }

    pub(crate) async fn upsert_mcp(&self, agent: String, config_json: String) -> Result<McpInfo> {
        anyhow::ensure!(!agent.is_empty(), "agent name is required for upsert_mcp");
        let cfg: wcore::McpServerConfig =
            serde_json::from_str(&config_json).context("invalid McpServerConfig JSON")?;
        anyhow::ensure!(!cfg.name.is_empty(), "MCP config must have a name");
        let mcp_name = cfg.name.clone();

        let rt = self.runtime.read().await.clone();
        let mut existing = rt
            .storage()
            .load_agent_by_name(&agent)
            .await?
            .ok_or_else(|| anyhow::anyhow!("agent '{agent}' not found"))?;
        let prompt = std::mem::take(&mut existing.system_prompt);
        if let Some(slot) = existing.mcps.iter_mut().find(|m| m.name == mcp_name) {
            *slot = cfg;
        } else {
            existing.mcps.push(cfg);
        }
        rt.update_agent(existing, &prompt).await?;

        // Re-list this agent to surface runtime status set by the
        // background register triggered through `on_register_agent`.
        let mcps = self.list_mcps(Some(agent)).await?;
        mcps.into_iter()
            .find(|m| m.name == mcp_name)
            .ok_or_else(|| anyhow::anyhow!("mcp '{mcp_name}' missing from listing after upsert"))
    }

    pub(crate) async fn delete_mcp(&self, agent: String, name: String) -> Result<bool> {
        anyhow::ensure!(!agent.is_empty(), "agent name is required for delete_mcp");
        let rt = self.runtime.read().await.clone();
        let mut existing = rt
            .storage()
            .load_agent_by_name(&agent)
            .await?
            .ok_or_else(|| anyhow::anyhow!("agent '{agent}' not found"))?;
        let prompt = std::mem::take(&mut existing.system_prompt);
        let before = existing.mcps.len();
        existing.mcps.retain(|m| m.name != name);
        if existing.mcps.len() == before {
            return Ok(false);
        }
        rt.update_agent(existing, &prompt).await?;
        // Phase 2 only: drop the bridge peer keyed by name. With
        // fingerprint refcounting (Phase 3) this becomes a refcount
        // decrement so peers shared across agents survive.
        self.mcp.disconnect_server(&name).await;
        Ok(true)
    }

    pub(crate) fn list_skills(&self) -> Vec<SkillInfo> {
        let dirs = wcore::resolve_dirs(&self.config_dir);
        let local_skills_dir = self.config_dir.join(wcore::paths::SKILLS_DIR);

        let dir_to_pkg: std::collections::BTreeMap<_, _> = dirs
            .plugin_skill_dirs
            .iter()
            .map(|(id, dir)| (dir.clone(), id.clone()))
            .collect();

        let mut seen = std::collections::BTreeSet::new();
        let mut skills = Vec::new();

        for dir in &dirs.skill_dirs {
            let (source, source_kind) = if *dir == local_skills_dir {
                ("local".to_string(), SourceKind::Local)
            } else if let Some(pkg_id) = dir_to_pkg.get(dir) {
                (pkg_id.clone(), SourceKind::Plugin)
            } else {
                let name = wcore::external_source_name(dir).unwrap_or("external");
                (name.to_string(), SourceKind::External)
            };

            for name in wcore::scan_skill_names(dir) {
                if !seen.insert(name.clone()) {
                    continue;
                }
                skills.push(SkillInfo {
                    name,
                    source: source.clone(),
                    source_kind: source_kind.into(),
                });
            }
        }
        skills
    }
}

fn mcp_info(
    cfg: &wcore::McpServerConfig,
    agent: &str,
    states: &BTreeMap<String, McpServerState>,
) -> McpInfo {
    let (status, tool_count, error) = match states.get(&cfg.name) {
        Some(state) => (
            proto_status(state.status),
            state.tools.len() as u32,
            state.last_error.clone().unwrap_or_default(),
        ),
        // Declared by the agent but not yet attempted (e.g., agent
        // recently registered, connect still scheduled).
        None => (McpStatus::Unknown, 0, String::new()),
    };
    McpInfo {
        name: cfg.name.clone(),
        command: cfg.command.clone(),
        args: cfg.args.clone(),
        env: cfg
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        url: cfg.url.clone().unwrap_or_default(),
        auth: cfg.auth,
        // The "source" is now the agent that owns the declaration.
        source: agent.to_string(),
        auto_restart: cfg.auto_restart,
        source_kind: SourceKind::Local.into(),
        status: status.into(),
        error,
        tool_count,
    }
}

fn proto_status(s: ServerStatus) -> McpStatus {
    match s {
        ServerStatus::Connecting => McpStatus::Connecting,
        ServerStatus::Connected => McpStatus::Connected,
        ServerStatus::Failed => McpStatus::Failed,
        ServerStatus::Disconnected => McpStatus::Disconnected,
    }
}
