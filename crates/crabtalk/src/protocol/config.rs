//! Configuration management: providers, models, MCPs, skills.

use crate::daemon::Daemon;
use anyhow::{Context, Result};
use crabllm_core::Provider;
use wcore::protocol::message::*;
use wcore::storage::Storage;

pub(super) async fn list_providers<P: Provider + 'static>(
    node: &Daemon<P>,
) -> Result<Vec<ProviderInfo>> {
    let config = load_config(node).await?;
    let active_model = active_model(node).await;
    Ok(config
        .provider
        .iter()
        .map(|(name, def)| {
            let cfg_json = serde_json::to_string(def).unwrap_or_default();
            let active = !active_model.is_empty() && def.models.contains(&active_model);
            ProviderInfo {
                name: name.clone(),
                active,
                config: cfg_json,
            }
        })
        .collect())
}

pub(super) async fn set_provider<P: Provider + 'static>(
    node: &Daemon<P>,
    name: String,
    config: String,
) -> Result<ProviderInfo> {
    let def: wcore::ProviderDef =
        serde_json::from_str(&config).context("invalid ProviderDef JSON")?;

    let rt = node.runtime.read().await.clone();
    let storage = rt.storage();
    let mut node_config = storage.load_config()?;
    node_config.provider.insert(name.clone(), def);
    wcore::validate_providers(&node_config.provider)?;
    storage.save_config(&node_config)?;
    node.reload().await?;

    let loaded_config = load_config(node).await?;
    let loaded_json = loaded_config
        .provider
        .get(&name)
        .and_then(|def| serde_json::to_string(def).ok())
        .unwrap_or_default();
    let active_model = active_model(node).await;
    let active = loaded_config
        .provider
        .get(&name)
        .is_some_and(|def| !active_model.is_empty() && def.models.contains(&active_model));
    Ok(ProviderInfo {
        name,
        active,
        config: loaded_json,
    })
}

pub(super) async fn delete_provider<P: Provider + 'static>(
    node: &Daemon<P>,
    name: String,
) -> Result<()> {
    let rt = node.runtime.read().await.clone();
    let storage = rt.storage();
    let mut config = storage.load_config()?;
    if config.provider.remove(&name).is_none() {
        anyhow::bail!("provider '{name}' not found");
    }
    storage.save_config(&config)?;
    node.reload().await
}

pub(super) async fn set_active_model<P: Provider + 'static>(
    node: &Daemon<P>,
    model: String,
) -> Result<()> {
    let rt = node.runtime.read().await.clone();
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
    node.reload().await
}

pub(super) async fn list_provider_presets() -> Result<Vec<ProviderPresetInfo>> {
    Ok(wcore::config::PROVIDER_PRESETS
        .iter()
        .map(|p| ProviderPresetInfo {
            name: p.name.to_string(),
            kind: ProviderKind::from(p.kind.clone()).into(),
            base_url: p.base_url.to_string(),
            fixed_base_url: p.fixed_base_url.to_string(),
            default_model: p.default_model.to_string(),
        })
        .collect())
}

pub(super) async fn list_mcps<P: Provider + 'static>(node: &Daemon<P>) -> Result<Vec<McpInfo>> {
    let connected: std::collections::BTreeMap<String, usize> = node
        .mcp
        .cached_list()
        .into_iter()
        .map(|(name, tools)| (name, tools.len()))
        .collect();

    let mut mcps = Vec::new();

    // Storage wins over manifest on name conflict.
    let storage_mcps = {
        let rt = node.runtime.read().await.clone();
        rt.storage().list_mcps()?
    };
    for (name, cfg) in &storage_mcps {
        let (status, tool_count) = mcp_status(&connected, name);
        mcps.push(mcp_to_info(
            name,
            cfg,
            "local",
            SourceKind::Local,
            status,
            tool_count,
        ));
    }

    for (plugin_name, plugin_manifest) in super::plugin::scan_plugin_manifests(&node.config_dir) {
        for (name, mcp_res) in &plugin_manifest.mcps {
            if mcps.iter().any(|m| m.name == *name) {
                continue;
            }
            let (status, tool_count) = mcp_status(&connected, name);
            let cfg = mcp_res.to_server_config();
            mcps.push(mcp_to_info(
                name,
                &cfg,
                &plugin_name,
                SourceKind::Plugin,
                status,
                tool_count,
            ));
        }
    }

    Ok(mcps)
}

pub(super) async fn upsert_mcp<P: Provider + 'static>(
    node: &Daemon<P>,
    config_json: String,
) -> Result<McpInfo> {
    let cfg: wcore::McpServerConfig =
        serde_json::from_str(&config_json).context("invalid McpServerConfig JSON")?;
    let name = cfg.name.clone();
    {
        let rt = node.runtime.read().await.clone();
        rt.storage().upsert_mcp(&cfg)?;
    }
    node.reload().await?;

    // Re-list to surface the runtime status (connected/failed/etc).
    let mcps = list_mcps(node).await?;
    mcps.into_iter()
        .find(|m| m.name == name)
        .ok_or_else(|| anyhow::anyhow!("mcp '{name}' missing from listing after upsert"))
}

pub(super) async fn delete_mcp<P: Provider + 'static>(
    node: &Daemon<P>,
    name: String,
) -> Result<bool> {
    let removed = {
        let rt = node.runtime.read().await.clone();
        rt.storage().delete_mcp(&name)?
    };
    if removed {
        node.reload().await?;
    }
    Ok(removed)
}

pub(super) async fn list_skills<P: Provider + 'static>(node: &Daemon<P>) -> Result<Vec<SkillInfo>> {
    let dirs = wcore::resolve_dirs(&node.config_dir);
    let local_skills_dir = node.config_dir.join(wcore::paths::SKILLS_DIR);

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
                source_kind: source_kind as i32,
            });
        }
    }
    Ok(skills)
}

pub(super) async fn list_models<P: Provider + 'static>(node: &Daemon<P>) -> Result<Vec<ModelInfo>> {
    let config = load_config(node).await?;
    let active_model = active_model(node).await;

    let mut models = Vec::new();
    for (provider_name, def) in &config.provider {
        let kind: i32 = ProviderKind::from(def.kind.clone()).into();
        for model_name in &def.models {
            models.push(ModelInfo {
                name: model_name.clone(),
                provider: provider_name.clone(),
                active: *model_name == active_model,
                kind,
            });
        }
    }
    Ok(models)
}

// ── Helpers shared across protocol handlers ──────────────────────────

pub(super) async fn load_config<P: Provider + 'static>(
    node: &Daemon<P>,
) -> Result<wcore::DaemonConfig> {
    let rt = node.runtime.read().await.clone();
    rt.storage().load_config()
}

/// Active model = the crab agent's `model` field. Empty string if the
/// crab agent is missing (which only happens before scaffold runs).
pub(super) async fn active_model<P: Provider + 'static>(node: &Daemon<P>) -> String {
    let rt = node.runtime.read().await.clone();
    rt.storage()
        .load_agent_by_name(wcore::paths::DEFAULT_AGENT)
        .ok()
        .flatten()
        .map(|c| c.model)
        .unwrap_or_default()
}

pub(super) async fn provider_name_for_model<P: Provider + 'static>(
    node: &Daemon<P>,
    model: &str,
) -> String {
    load_config(node)
        .await
        .ok()
        .and_then(|c| {
            c.provider
                .iter()
                .find(|(_, def)| def.models.iter().any(|m| m == model))
                .map(|(name, _)| name.clone())
        })
        .unwrap_or_default()
}

fn mcp_status(
    connected: &std::collections::BTreeMap<String, usize>,
    name: &str,
) -> (McpStatus, u32) {
    match connected.get(name) {
        Some(&count) => (McpStatus::Connected, count as u32),
        None => (McpStatus::Failed, 0),
    }
}

fn mcp_to_info(
    name: &str,
    cfg: &wcore::McpServerConfig,
    source: &str,
    source_kind: SourceKind,
    status: McpStatus,
    tool_count: u32,
) -> McpInfo {
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
