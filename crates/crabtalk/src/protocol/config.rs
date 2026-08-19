//! Configuration mutations: active model, MCP, skills.

use crate::{llm::Provider, system::CrabTalk};
use anyhow::{Context, Result};
use mcp::McpServerState;
use proto::*;
use std::collections::BTreeMap;
use store::{AgentConfig, AgentId, interface::Backend};

/// How many skills one `list_skills` answers with. A catalogue can be
/// large and every entry is only a name and a description here — the
/// body is `get_skill`, for the one an agent actually invokes.
const SKILL_PAGE: usize = 200;

impl<P: Provider + 'static, S: Backend> CrabTalk<P, S> {
    pub(crate) async fn set_active_model(&self, model: String) -> Result<()> {
        let rt = self.runtime.read().await.clone();
        let known = rt.list_models().await;
        if !known.is_empty() && !known.iter().any(|m| m.name == model) {
            anyhow::bail!("model '{model}' not advertised by the LLM endpoint");
        }

        // The next run resolves the agent fresh from storage, so the
        // swap takes effect without rebuilding the daemon.
        let mut default = match rt.default_agent().await {
            Some(config) => config,
            None => AgentConfig::crab(&model),
        };
        default.model = model;
        let id = default.id;
        rt.update_agent(&id, default).await?;
        Ok(())
    }

    pub(crate) async fn list_mcps(&self, agent: Option<AgentId>) -> Result<Vec<McpInfo>> {
        let states = self.registry.mcp.handler.states();
        let rt = self.runtime.read().await.clone();
        let mut out: Vec<McpInfo> = Vec::new();
        let configs = match agent {
            Some(id) => vec![
                rt.agent(&id)
                    .await
                    .ok_or_else(|| anyhow::anyhow!("agent '{id}' not found"))?,
            ],
            None => rt.agents().await,
        };
        for cfg in configs {
            for mcp_cfg in &cfg.mcps {
                out.push(mcp_info(mcp_cfg, &cfg, &states));
            }
        }
        Ok(out)
    }

    pub(crate) async fn upsert_mcp(&self, agent: AgentId, config_json: String) -> Result<McpInfo> {
        let cfg: store::McpServerConfig =
            serde_json::from_str(&config_json).context("invalid McpServerConfig JSON")?;
        anyhow::ensure!(!cfg.name.is_empty(), "MCP config must have a name");
        let mcp_name = cfg.name.clone();

        let rt = self.runtime.read().await.clone();
        let mut existing = rt
            .storage()
            .load_agent(&agent)
            .await?
            .ok_or_else(|| anyhow::anyhow!("agent '{agent}' not found"))?;
        if let Some(slot) = existing.mcps.iter_mut().find(|m| m.name == mcp_name) {
            *slot = cfg;
        } else {
            existing.mcps.push(cfg);
        }
        rt.update_agent(&agent, existing).await?;
        self.registry
            .mcp
            .handler
            .ensure_connected(&agent.to_string(), std::slice::from_ref(&mcp_name))
            .await;

        let mcps = self.list_mcps(Some(agent)).await?;
        mcps.into_iter()
            .find(|m| m.name == mcp_name)
            .ok_or_else(|| anyhow::anyhow!("mcp '{mcp_name}' missing from listing after upsert"))
    }

    pub(crate) async fn delete_mcp(&self, agent: AgentId, name: String) -> Result<bool> {
        let rt = self.runtime.read().await.clone();
        let mut existing = rt
            .storage()
            .load_agent(&agent)
            .await?
            .ok_or_else(|| anyhow::anyhow!("agent '{agent}' not found"))?;
        let before = existing.mcps.len();
        existing.mcps.retain(|m| m.name != name);
        if existing.mcps.len() == before {
            return Ok(false);
        }
        rt.update_agent(&agent, existing).await?;
        Ok(true)
    }

    /// Respawn the peer behind an agent's MCP. Nothing on disk changes —
    /// the handler reconnects from the config the peer is already running,
    /// so this is the answer to a dead connection, not to a stale one.
    pub(crate) async fn reconnect_mcp(&self, agent: AgentId, name: String) -> Result<McpInfo> {
        self.registry
            .mcp
            .handler
            .reconnect_for_agent(&agent.to_string(), &name)
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
        let rt = self.runtime.read().await.clone();
        // One page. A catalogue is not something a listing reads whole:
        // `SKILL_PAGE` bounds what crosses the wire, and anything past it
        // is a second call rather than a silent truncation of the answer.
        match rt.storage().list_skills(SKILL_PAGE, 0).await {
            Ok(skills) => skills
                .into_iter()
                .map(|s| SkillInfo {
                    name: s.name,
                    description: s.description,
                })
                .collect(),
            Err(e) => {
                tracing::warn!("failed to list skills: {e}");
                Vec::new()
            }
        }
    }
}

/// One MCP as the wire describes it. Peers are keyed by the agent's
/// ULID — that is the scope key `lib/mcp` was handed — while `source`
/// carries the name, because this listing is read.
fn mcp_info(
    cfg: &store::McpServerConfig,
    agent: &store::AgentConfig,
    states: &BTreeMap<(String, String), McpServerState>,
) -> McpInfo {
    let key = (agent.id.to_string(), cfg.name.clone());
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
        source: agent.name.clone(),
        auto_restart: cfg.auto_restart,
        source_kind: SourceKind::Local.into(),
        status: status.into(),
        error,
        tool_count,
    }
}
