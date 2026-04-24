//! Daemon-level configuration mutations: provider / model / MCP / skill.
//! Pure storage-backed queries live on `Runtime<C>` directly.

use crate::daemon::Daemon;
use anyhow::{Context, Result};
use crabllm_core::Provider;
use wcore::protocol::message::*;
use wcore::storage::Storage;

impl<P: Provider + 'static> Daemon<P> {
    pub(crate) async fn set_provider(&self, name: String, config: String) -> Result<ProviderInfo> {
        let def: wcore::ProviderDef =
            serde_json::from_str(&config).context("invalid ProviderDef JSON")?;
        let rt = self.runtime.read().await.clone();
        let storage = rt.storage();
        let mut node_config = storage.load_config()?;
        node_config.provider.insert(name.clone(), def);
        wcore::validate_providers(&node_config.provider)?;
        storage.save_config(&node_config)?;
        self.reload().await?;

        let rt = self.runtime.read().await.clone();
        rt.list_providers()?
            .into_iter()
            .find(|p| p.name == name)
            .ok_or_else(|| anyhow::anyhow!("provider '{name}' missing after configure"))
    }

    pub(crate) async fn delete_provider(&self, name: &str) -> Result<()> {
        let rt = self.runtime.read().await.clone();
        let storage = rt.storage();
        let mut config = storage.load_config()?;
        if config.provider.remove(name).is_none() {
            anyhow::bail!("provider '{name}' not found");
        }
        storage.save_config(&config)?;
        self.reload().await
    }

    pub(crate) async fn set_active_model(&self, model: String) -> Result<()> {
        let rt = self.runtime.read().await.clone();
        let storage = rt.storage();

        let config = storage.load_config()?;
        let model_exists = config
            .provider
            .values()
            .any(|def| def.models.iter().any(|m| m == &model));
        if !model_exists {
            anyhow::bail!("model '{model}' not found in any provider");
        }

        let mut crab = storage
            .load_agent_by_name(wcore::paths::DEFAULT_AGENT)?
            .unwrap_or_else(|| crate::storage::default_crab(&model));
        let prompt = std::mem::take(&mut crab.system_prompt);
        crab.model = model;
        storage.upsert_agent(&crab, &prompt)?;
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
            rt.storage().list_mcps()?
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
            rt.storage().upsert_mcp(&cfg)?;
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
            rt.storage().delete_mcp(name)?
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

pub(super) fn provider_presets() -> Vec<ProviderPresetInfo> {
    wcore::config::PROVIDER_PRESETS
        .iter()
        .map(|p| ProviderPresetInfo {
            name: p.name.to_string(),
            kind: ProviderKind::from(&p.kind).into(),
            base_url: p.base_url.to_string(),
            fixed_base_url: p.fixed_base_url.to_string(),
            default_model: p.default_model.to_string(),
        })
        .collect()
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
