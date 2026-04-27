//! Daemon-level configuration mutations: active model, MCP, skills.

use crate::daemon::Daemon;
use anyhow::{Context, Result};
use crabllm_core::Provider;
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

    pub(crate) async fn list_mcps(&self) -> Result<Vec<McpInfo>> {
        let connected: std::collections::BTreeMap<String, usize> = self
            .mcp
            .cached_list()
            .into_iter()
            .map(|(name, tools)| (name, tools.len()))
            .collect();
        let storage_mcps = {
            let rt = self.runtime.read().await.clone();
            rt.storage().list_mcps().await?
        };
        // Storage wins over manifest on name conflict — seed the map from
        // storage first, then `entry(..).or_insert_with` for manifest entries
        // skips names already present. Output is alphabetical by name.
        let mut by_name: std::collections::BTreeMap<String, McpInfo> = storage_mcps
            .iter()
            .map(|(name, cfg)| {
                (
                    name.clone(),
                    mcp_info(name, cfg, "local", SourceKind::Local, &connected),
                )
            })
            .collect();
        for (plugin_name, manifest) in super::plugin::scan_plugin_manifests(&self.config_dir) {
            for (name, mcp_res) in manifest.mcps {
                by_name.entry(name.clone()).or_insert_with(|| {
                    mcp_info(
                        &name,
                        &mcp_res.to_server_config(),
                        &plugin_name,
                        SourceKind::Plugin,
                        &connected,
                    )
                });
            }
        }
        Ok(by_name.into_values().collect())
    }

    pub(crate) async fn upsert_mcp(&self, config_json: String) -> Result<McpInfo> {
        let cfg: wcore::McpServerConfig =
            serde_json::from_str(&config_json).context("invalid McpServerConfig JSON")?;
        let name = cfg.name.clone();
        {
            let rt = self.runtime.read().await.clone();
            rt.storage().upsert_mcp(&cfg).await?;
        }
        self.reload().await?;

        // Re-list to surface the runtime status (connected/failed/etc).
        let mcps = self.list_mcps().await?;
        mcps.into_iter()
            .find(|m| m.name == name)
            .ok_or_else(|| anyhow::anyhow!("mcp '{name}' missing from listing after upsert"))
    }

    pub(crate) async fn delete_mcp(&self, name: &str) -> Result<bool> {
        let removed = {
            let rt = self.runtime.read().await.clone();
            rt.storage().delete_mcp(name).await?
        };
        if removed {
            self.reload().await?;
        }
        Ok(removed)
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
    name: &str,
    cfg: &wcore::McpServerConfig,
    source: &str,
    source_kind: SourceKind,
    connected: &std::collections::BTreeMap<String, usize>,
) -> McpInfo {
    let (status, tool_count) = match connected.get(name) {
        Some(&count) => (McpStatus::Connected, count as u32),
        None => (McpStatus::Failed, 0),
    };
    McpInfo {
        name: name.to_string(),
        command: cfg.command.clone(),
        args: cfg.args.clone(),
        env: cfg
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        url: cfg.url.clone().unwrap_or_default(),
        auth: cfg.auth,
        source: source.to_string(),
        auto_restart: cfg.auto_restart,
        source_kind: source_kind.into(),
        status: status.into(),
        error: String::new(),
        tool_count,
    }
}
