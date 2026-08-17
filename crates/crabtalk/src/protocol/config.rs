//! Configuration mutations: active model, MCP, skills.

use crate::{llm::Provider, system::CrabTalk};
use anyhow::{Context, Result};
use mcp::McpServerState;
use std::collections::BTreeMap;
use wcore::{AgentConfig, protocol::message::*, storage::Storage};

impl<P: Provider + 'static, S: Storage> CrabTalk<P, S> {
    pub(crate) async fn set_active_model(&self, model: String) -> Result<()> {
        let rt = self.runtime.read().await.clone();
        let storage = rt.storage();
        let known = rt.list_models().await;
        if !known.is_empty() && !known.iter().any(|m| m.name == model) {
            anyhow::bail!("model '{model}' not advertised by the LLM endpoint");
        }

        let mut crab = storage
            .load_agent_by_name(wcore::paths::DEFAULT_AGENT)
            .await?
            .unwrap_or_else(|| AgentConfig::crab(&model));
        crab.model = model;
        storage.upsert_agent(&crab).await?;
        self.reload().await
    }

    pub(crate) async fn list_mcps(&self, agent: Option<String>) -> Result<Vec<McpInfo>> {
        let states = self.mcp.states();
        let rt = self.runtime.read().await.clone();
        let mut out: Vec<McpInfo> = Vec::new();
        match agent {
            Some(name) => {
                let cfg = rt
                    .agent(&name)
                    .ok_or_else(|| anyhow::anyhow!("agent '{name}' not found"))?;
                for mcp_cfg in &cfg.mcps {
                    out.push(mcp_info(mcp_cfg, &name, &states));
                }
            }
            None => {
                for cfg in rt.agents() {
                    for mcp_cfg in &cfg.mcps {
                        out.push(mcp_info(mcp_cfg, &cfg.name, &states));
                    }
                }
            }
        }
        Ok(out)
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
        if let Some(slot) = existing.mcps.iter_mut().find(|m| m.name == mcp_name) {
            *slot = cfg;
        } else {
            existing.mcps.push(cfg);
        }
        rt.update_agent(existing).await?;
        self.mcp
            .ensure_connected(&agent, std::slice::from_ref(&mcp_name))
            .await;

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
        let before = existing.mcps.len();
        existing.mcps.retain(|m| m.name != name);
        if existing.mcps.len() == before {
            return Ok(false);
        }
        rt.update_agent(existing).await?;
        Ok(true)
    }

    /// Respawn the peer behind an agent's MCP. Nothing on disk changes —
    /// the handler reconnects from the config the peer is already running,
    /// so this is the answer to a dead connection, not to a stale one.
    pub(crate) async fn reconnect_mcp(&self, agent: String, name: String) -> Result<McpInfo> {
        anyhow::ensure!(
            !agent.is_empty(),
            "agent name is required for reconnect_mcp"
        );
        self.mcp
            .reconnect_for_agent(&agent, &name)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        let mcps = self.list_mcps(Some(agent)).await?;
        mcps.into_iter()
            .find(|m| m.name == name)
            .ok_or_else(|| anyhow::anyhow!("mcp '{name}' missing from listing after reconnect"))
    }

    /// One skill's instructions, for a client or a harness that has chosen
    /// from the catalogue.
    pub(crate) async fn get_skill(&self, name: String) -> Result<SkillBody> {
        let rt = self.runtime.read().await.clone();
        let skill = rt
            .storage()
            .load_skill(&name)
            .await?
            .with_context(|| format!("no skill named '{name}'"))?;
        Ok(SkillBody {
            name: skill.name,
            body: skill.body,
        })
    }

    pub(crate) async fn list_skills(&self) -> Vec<SkillInfo> {
        let dirs = wcore::resolve_dirs(&self.config_dir);
        let local_skills_dir = self.config_dir.join(wcore::paths::SKILLS_DIR);
        let described: BTreeMap<String, String> = match self
            .runtime
            .read()
            .await
            .clone()
            .storage()
            .list_skills()
            .await
        {
            Ok(skills) => skills
                .into_iter()
                .map(|s| (s.name, s.description))
                .collect(),
            Err(e) => {
                tracing::warn!("failed to read skill descriptions: {e}");
                BTreeMap::new()
            }
        };

        let dir_to_pkg: std::collections::BTreeMap<_, _> = dirs
            .package_skill_dirs
            .iter()
            .map(|(id, dir)| (dir.clone(), id.clone()))
            .collect();

        let mut seen = std::collections::BTreeSet::new();
        let mut skills = Vec::new();
        for dir in &dirs.skill_dirs {
            let (source, source_kind) = if *dir == local_skills_dir {
                ("local".to_string(), SourceKind::Local)
            } else if let Some(pkg_id) = dir_to_pkg.get(dir) {
                (pkg_id.clone(), SourceKind::Package)
            } else {
                let name = wcore::external_source_name(dir).unwrap_or("external");
                (name.to_string(), SourceKind::External)
            };

            for name in skill::discover::scan_names(dir) {
                if !seen.insert(name.clone()) {
                    continue;
                }
                let description = described.get(&name).cloned().unwrap_or_default();
                skills.push(SkillInfo {
                    name,
                    source: source.clone(),
                    source_kind: source_kind.into(),
                    description,
                });
            }
        }
        skills
    }
}

fn mcp_info(
    cfg: &wcore::McpServerConfig,
    agent: &str,
    states: &BTreeMap<(String, String), McpServerState>,
) -> McpInfo {
    let key = (agent.to_owned(), cfg.name.clone());
    let (status, tool_count, error) = match states.get(&key) {
        Some(state) => (
            state.status.into(),
            state.tools.len() as u32,
            state.last_error.clone().unwrap_or_default(),
        ),
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
        auth: cfg.auth.clone().unwrap_or_default(),
        source: agent.to_string(),
        auto_restart: cfg.auto_restart,
        source_kind: SourceKind::Local.into(),
        status: status.into(),
        error,
        tool_count,
    }
}
