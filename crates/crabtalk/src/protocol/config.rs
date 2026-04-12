//! Configuration management: providers, models, MCPs, skills, enabled state.

use crate::node::Node;
use anyhow::{Context, Result};
use crabllm_core::Provider;
use runtime::host::Host;
use wcore::protocol::message::*;
use wcore::storage::Storage;

pub(super) async fn list_providers<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
) -> Result<Vec<ProviderInfo>> {
    let config = load_config(node).await?;
    let (manifest, _) = resolve_manifests(node).await?;
    let active_model = config.system.crab.model.clone().unwrap_or_default();
    Ok(config
        .provider
        .iter()
        .map(|(name, def)| {
            let cfg_json = serde_json::to_string(def).unwrap_or_default();
            let active = !active_model.is_empty() && def.models.contains(&active_model);
            let enabled = !manifest.disabled.providers.contains(name);
            ProviderInfo {
                name: name.clone(),
                active,
                config: cfg_json,
                enabled,
            }
        })
        .collect())
}

pub(super) async fn set_provider<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
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
    let active_model = loaded_config.system.crab.model.clone().unwrap_or_default();
    let active = loaded_config
        .provider
        .get(&name)
        .is_some_and(|def| !active_model.is_empty() && def.models.contains(&active_model));
    let enabled = !loaded_config.disabled.providers.contains(&name);
    Ok(ProviderInfo {
        name,
        active,
        config: loaded_json,
        enabled,
    })
}

pub(super) async fn delete_provider<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
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

pub(super) async fn set_active_model<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
    model: String,
) -> Result<()> {
    let rt = node.runtime.read().await.clone();
    let storage = rt.storage();
    let mut config = storage.load_config()?;

    let model_exists = config
        .provider
        .values()
        .any(|def| def.models.iter().any(|m| m == &model));
    if !model_exists {
        anyhow::bail!("model '{model}' not found in any provider");
    }

    config.system.crab.model = Some(model);
    storage.save_config(&config)?;
    node.reload().await
}

pub(super) async fn list_provider_presets() -> Result<Vec<ProviderPresetInfo>> {
    Ok(wcore::config::PROVIDER_PRESETS
        .iter()
        .map(|p| ProviderPresetInfo {
            name: p.name.to_string(),
            kind: ProtoProviderKind::from(p.kind).into(),
            base_url: p.base_url.to_string(),
            fixed_base_url: p.fixed_base_url.to_string(),
            default_model: p.default_model.to_string(),
        })
        .collect())
}

pub(super) async fn list_mcps<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
) -> Result<Vec<McpInfo>> {
    let config = load_config(node).await?;
    let connected: std::collections::BTreeMap<String, usize> = node
        .mcp
        .cached_list()
        .into_iter()
        .map(|(name, tools)| (name, tools.len()))
        .collect();

    let mut mcps = Vec::new();

    let manifest_path = node
        .config_dir
        .join(wcore::paths::LOCAL_DIR)
        .join("CrabTalk.toml");
    if let Ok(Some(local)) = wcore::ManifestConfig::load(&manifest_path) {
        for (name, cfg) in &local.mcps {
            let enabled = !config.disabled.mcps.contains(name);
            let (status, tool_count) = mcp_status(&connected, name, enabled);
            mcps.push(mcp_to_info(
                name,
                cfg,
                "local",
                SourceKind::Local,
                enabled,
                status,
                tool_count,
            ));
        }
    }

    for (plugin_name, plugin_manifest) in super::plugin::scan_plugin_manifests(&node.config_dir) {
        for (name, mcp_res) in &plugin_manifest.mcps {
            if mcps.iter().any(|m| m.name == *name) {
                continue;
            }
            let enabled = !config.disabled.mcps.contains(name);
            let (status, tool_count) = mcp_status(&connected, name, enabled);
            let cfg = mcp_res.to_server_config();
            mcps.push(mcp_to_info(
                name,
                &cfg,
                &plugin_name,
                SourceKind::Plugin,
                enabled,
                status,
                tool_count,
            ));
        }
    }

    Ok(mcps)
}

pub(super) async fn set_local_mcps<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
    mcps: Vec<McpInfo>,
) -> Result<()> {
    let rt = node.runtime.read().await.clone();
    let storage = rt.storage();
    let mut manifest = storage.load_local_manifest()?;
    manifest.mcps.clear();
    for mcp in mcps {
        let config = wcore::McpServerConfig {
            name: mcp.name.clone(),
            command: mcp.command,
            args: mcp.args,
            env: mcp.env.into_iter().collect(),
            auto_restart: mcp.auto_restart,
            url: if mcp.url.is_empty() {
                None
            } else {
                Some(mcp.url)
            },
            auth: mcp.auth,
        };
        manifest.mcps.insert(mcp.name, config);
    }
    storage.save_local_manifest(&manifest)?;
    node.reload().await
}

pub(super) async fn list_skills<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
) -> Result<Vec<SkillInfo>> {
    let (manifest, _) = resolve_manifests(node).await?;
    let local_skills_dir = node.config_dir.join(wcore::paths::SKILLS_DIR);

    let dir_to_pkg: std::collections::BTreeMap<_, _> = manifest
        .plugin_skill_dirs
        .iter()
        .map(|(id, dir)| (dir.clone(), id.clone()))
        .collect();

    let mut seen = std::collections::BTreeSet::new();
    let mut skills = Vec::new();

    for dir in &manifest.skill_dirs {
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
            let enabled = !manifest.disabled.skills.contains(&name)
                && (source_kind != SourceKind::External
                    || !manifest.disabled.external.contains(&source));
            skills.push(SkillInfo {
                name,
                enabled,
                source: source.clone(),
                source_kind: source_kind as i32,
            });
        }
    }
    Ok(skills)
}

pub(super) async fn list_models<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
) -> Result<Vec<ModelInfo>> {
    let config = load_config(node).await?;
    let active_model = config.system.crab.model.clone().unwrap_or_default();

    let mut models = Vec::new();
    for (provider_name, def) in &config.provider {
        let enabled = !config.disabled.providers.contains(provider_name);
        let kind: i32 = ProtoProviderKind::from(def.kind).into();
        for model_name in &def.models {
            models.push(ModelInfo {
                name: model_name.clone(),
                provider: provider_name.clone(),
                active: *model_name == active_model,
                enabled,
                kind,
            });
        }
    }
    Ok(models)
}

pub(super) async fn set_enabled<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
    kind: ResourceKind,
    name: String,
    enabled: bool,
) -> Result<()> {
    let rt = node.runtime.read().await.clone();
    let storage = rt.storage();
    let mut config = storage.load_config()?;

    if !enabled && kind == ResourceKind::Provider {
        let active_model = config.system.crab.model.clone().unwrap_or_default();
        if !active_model.is_empty()
            && config
                .provider
                .get(&name)
                .is_some_and(|def| def.models.contains(&active_model))
        {
            anyhow::bail!(
                "cannot disable provider '{name}' — it serves the active model '{active_model}'"
            );
        }
    }

    let list = match kind {
        ResourceKind::Provider => &mut config.disabled.providers,
        ResourceKind::Mcp => &mut config.disabled.mcps,
        ResourceKind::Skill => &mut config.disabled.skills,
        ResourceKind::ExternalSource => &mut config.disabled.external,
        ResourceKind::Unknown => anyhow::bail!("unknown resource kind"),
    };
    if enabled {
        list.retain(|v| v != &name);
    } else if !list.contains(&name) {
        list.push(name);
    }

    storage.save_config(&config)?;
    node.reload().await
}

// ── Helpers shared across protocol handlers ──────────────────────────

pub(super) async fn load_config<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
) -> Result<wcore::NodeConfig> {
    let rt = node.runtime.read().await.clone();
    rt.storage().load_config()
}

pub(super) async fn provider_name_for_model<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
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

pub(super) async fn resolve_manifests<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
) -> Result<(wcore::ResolvedManifest, Vec<String>)> {
    let config = load_config(node).await?;
    let (mut manifest, warnings) = wcore::resolve_manifests(&node.config_dir);
    manifest.disabled = config.disabled;
    wcore::filter_disabled_external(&mut manifest.skill_dirs, &manifest.disabled.external);
    Ok((manifest, warnings))
}

fn mcp_status(
    connected: &std::collections::BTreeMap<String, usize>,
    name: &str,
    enabled: bool,
) -> (McpStatus, u32) {
    if !enabled {
        return (McpStatus::Disconnected, 0);
    }
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
    enabled: bool,
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
        enabled,
        source_kind: source_kind.into(),
        status: status.into(),
        error: String::new(),
        tool_count,
    }
}
