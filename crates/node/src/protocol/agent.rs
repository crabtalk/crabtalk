//! Agent CRUD: list, get, create, update, delete.

use crate::node::Node;
use anyhow::{Context, Result};
use crabllm_core::Provider;
use runtime::host::Host;
use wcore::protocol::message::*;
use wcore::repos::Storage;

pub(super) async fn list<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
) -> Result<Vec<AgentInfo>> {
    let rt = node.runtime.read().await.clone();
    Ok(rt
        .agents()
        .into_iter()
        .map(|c| agent_config_to_info(&c))
        .collect())
}

pub(super) async fn get<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
    name: String,
) -> Result<AgentInfo> {
    let rt = node.runtime.read().await.clone();
    let config = rt
        .agent(&name)
        .ok_or_else(|| anyhow::anyhow!("agent '{name}' not found"))?;
    Ok(agent_config_to_info(&config))
}

pub(super) async fn create<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
    req: CreateAgentMsg,
) -> Result<AgentInfo> {
    validate_agent_name(&req.name)?;
    let mut config: wcore::AgentConfig =
        serde_json::from_str(&req.config).context("invalid AgentConfig JSON")?;
    if config.id.is_nil() {
        config.id = wcore::AgentId::new();
    }
    let id = config.id;
    let normalized =
        serde_json::to_string(&config).context("failed to re-serialize normalized agent config")?;
    write_agent_to_manifest(node, &req.name, &normalized, true).await?;
    write_agent_prompt_to_storage(node, &id, &req.prompt).await?;
    register_agent_from_disk(node, &req.name).await?;
    get(node, req.name).await
}

pub(super) async fn update<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
    req: UpdateAgentMsg,
) -> Result<AgentInfo> {
    validate_agent_name(&req.name)?;
    if req.name == wcore::paths::DEFAULT_AGENT {
        write_system_crab_config(node, &req.config).await?;
    } else {
        let mut config: wcore::AgentConfig =
            serde_json::from_str(&req.config).context("invalid AgentConfig JSON")?;
        let existing = existing_agent_id(node, &req.name).await?;
        config.id = existing.unwrap_or_else(|| {
            if config.id.is_nil() {
                wcore::AgentId::new()
            } else {
                config.id
            }
        });
        let id = config.id;
        let normalized = serde_json::to_string(&config)
            .context("failed to re-serialize normalized agent config")?;
        write_agent_to_manifest(node, &req.name, &normalized, false).await?;
        if !req.prompt.is_empty() {
            write_agent_prompt_to_storage(node, &id, &req.prompt).await?;
        }
    }
    register_agent_from_disk(node, &req.name).await?;
    get(node, req.name).await
}

pub(super) async fn delete<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
    name: String,
) -> Result<bool> {
    let rt = node.runtime.read().await.clone();
    let storage = rt.storage();

    let mut manifest = storage.load_local_manifest()?;
    let existing_id = manifest
        .agents
        .get(&name)
        .filter(|c| !c.id.is_nil())
        .map(|c| c.id);
    let removed = manifest.agents.remove(&name).is_some();
    if removed {
        storage.save_local_manifest(&manifest)?;

        if let Some(id) = existing_id
            && let Err(e) = storage.delete_agent(&id)
        {
            tracing::warn!("failed to delete agent prompt for {id}: {e}");
        }

        rt.remove_agent(&name);
        rt.hook.unregister_scope(&name);
    }
    Ok(removed)
}

async fn register_agent_from_disk<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
    name: &str,
) -> Result<()> {
    let config = super::config::load_config(node).await?;
    let (manifest, _warnings) = super::config::resolve_manifests(node).await?;
    let rt = node.runtime.read().await.clone();
    let agent_config =
        crate::builder::build_single_agent_config(name, &config, &manifest, rt.storage().as_ref())?;
    let registered = rt.upsert_agent(agent_config);
    rt.hook.register_scope(name.to_owned(), &registered);
    Ok(())
}

async fn existing_agent_id<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
    name: &str,
) -> Result<Option<wcore::AgentId>> {
    let rt = node.runtime.read().await.clone();
    let manifest = rt.storage().load_local_manifest()?;
    Ok(manifest
        .agents
        .get(name)
        .filter(|cfg| !cfg.id.is_nil())
        .map(|cfg| cfg.id))
}

async fn write_agent_to_manifest<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
    name: &str,
    config_json: &str,
    expect_new: bool,
) -> Result<()> {
    let config: wcore::AgentConfig =
        serde_json::from_str(config_json).context("invalid AgentConfig JSON")?;

    let rt = node.runtime.read().await.clone();
    let storage = rt.storage();
    let mut manifest = storage.load_local_manifest()?;

    if expect_new && manifest.agents.contains_key(name) {
        anyhow::bail!("agent '{name}' already exists in local manifest");
    }

    manifest.agents.insert(name.to_owned(), config);
    storage.save_local_manifest(&manifest)?;
    Ok(())
}

async fn write_agent_prompt_to_storage<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
    id: &wcore::AgentId,
    prompt: &str,
) -> Result<()> {
    let rt = node.runtime.read().await.clone();
    let config = wcore::AgentConfig {
        id: *id,
        ..Default::default()
    };
    rt.storage()
        .upsert_agent(&config, prompt)
        .with_context(|| format!("failed to write agent prompt for {id}"))
}

async fn write_system_crab_config<P: Provider + 'static, H: Host + 'static>(
    node: &Node<P, H>,
    config_json: &str,
) -> Result<()> {
    let crab: wcore::AgentConfig =
        serde_json::from_str(config_json).context("invalid AgentConfig JSON")?;
    let rt = node.runtime.read().await.clone();
    let storage = rt.storage();
    let mut config = storage.load_config()?;
    config.system.crab = crab;
    storage.save_config(&config)
}

fn validate_agent_name(name: &str) -> Result<()> {
    anyhow::ensure!(!name.is_empty(), "agent name cannot be empty");
    anyhow::ensure!(
        !name.contains('/') && !name.contains('\\') && !name.contains(".."),
        "agent name '{name}' contains invalid characters"
    );
    Ok(())
}

fn agent_config_to_info(config: &wcore::AgentConfig) -> AgentInfo {
    AgentInfo {
        name: config.name.clone(),
        description: config.description.clone(),
        config: String::new(),
        model: config.model.clone(),
        max_iterations: config.max_iterations as u32,
        thinking: config.thinking,
        members: config.members.clone(),
        skills: config.skills.clone(),
        mcps: config.mcps.clone(),
        compact_threshold: config.compact_threshold.map(|t| t as u32),
        compact_tool_max_len: config.compact_tool_max_len as u32,
    }
}
