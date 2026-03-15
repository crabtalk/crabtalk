//! Walrus hub package install/uninstall operations.

use crate::ext::hub::{DownloadRegistry, manifest};
use anyhow::{Context, Result};
use compact_str::CompactString;
use std::path::Path;
use tokio::process::Command;
use wcore::paths::CONFIG_DIR;
use wcore::protocol::message::{
    DownloadCompleted, DownloadCreated, DownloadEvent, DownloadKind, DownloadStep, download_event,
};

/// Remote URL of the walrus hub repository.
const WALRUS_HUB: &str = "https://github.com/openwalrus/hub";

/// Install a hub package, streaming unified download events.
///
/// Syncs the hub repo, reads the manifest, merges MCP servers and services
/// into `walrus.toml`, copies skill directories into `~/.openwalrus/skills/`,
/// and installs agents (prompt files + their declared skills).
pub fn install(
    package: CompactString,
    registry: std::sync::Arc<tokio::sync::Mutex<DownloadRegistry>>,
) -> impl futures_core::Stream<Item = Result<DownloadEvent>> {
    async_stream::try_stream! {
        let id = registry
            .lock()
            .await
            .start(DownloadKind::Hub, package.to_string());
        yield DownloadEvent {
            event: Some(download_event::Event::Created(DownloadCreated {
                id,
                kind: DownloadKind::Hub as i32,
                label: package.to_string(),
            })),
        };

        // Sync hub repo (clone or update).
        let hub_dir = CONFIG_DIR.join("hub");
        git_sync(WALRUS_HUB, &hub_dir).await.context("failed to sync hub repo")?;

        let (scope, name) = parse_package(&package)?;
        let manifest = read_manifest(scope, name)?;

        // Merge MCP servers.
        if !manifest.mcp_servers.is_empty() {
            let msg = "adding MCP servers…".to_string();
            registry.lock().await.step(id, msg.clone());
            yield DownloadEvent {
                event: Some(download_event::Event::Step(DownloadStep { id, message: msg })),
            };
            merge_mcp_servers(&manifest)?;
        }

        // Install service binaries and merge configs.
        if !manifest.services.is_empty() {
            for (key, cfg) in &manifest.services {
                if let Some(install) = &cfg.install {
                    let msg = format!("installing {key}…");
                    registry.lock().await.step(id, msg.clone());
                    yield DownloadEvent {
                        event: Some(download_event::Event::Step(DownloadStep { id, message: msg })),
                    };
                    let status = Command::new(&install.command)
                        .args(&install.args)
                        .status()
                        .await
                        .with_context(|| format!("failed to run install for {key}"))?;
                    if !status.success() {
                        Err(anyhow::anyhow!("install for {key} exited with {status}"))?;
                    }
                }
            }

            let msg = "adding services…".to_string();
            registry.lock().await.step(id, msg.clone());
            yield DownloadEvent {
                event: Some(download_event::Event::Step(DownloadStep { id, message: msg })),
            };
            merge_services(&manifest)?;
        }

        // Collect skill keys referenced by agents so they are installed too.
        let mut agent_skill_keys: std::collections::BTreeSet<&CompactString> =
            std::collections::BTreeSet::new();
        for agent in manifest.agents.values() {
            for sk in &agent.skills {
                agent_skill_keys.insert(sk);
            }
        }

        // Install skills (top-level + agent-referenced).
        let all_skill_keys: std::collections::BTreeSet<&CompactString> = manifest
            .skills
            .keys()
            .chain(agent_skill_keys.iter().copied())
            .collect();

        if !all_skill_keys.is_empty() {
            let msg = "installing skills…".to_string();
            registry.lock().await.step(id, msg.clone());
            yield DownloadEvent {
                event: Some(download_event::Event::Step(DownloadStep { id, message: msg })),
            };
            let cache_dir = CONFIG_DIR.join(".cache").join("skills");
            let skills_dir = CONFIG_DIR.join("skills");
            std::fs::create_dir_all(&cache_dir).context("failed to create skill cache dir")?;
            std::fs::create_dir_all(&skills_dir).context("failed to create skills dir")?;

            for key in &all_skill_keys {
                let skill = manifest.skills.get(*key).ok_or_else(|| {
                    anyhow::anyhow!("agent references skill '{key}' not found in [skills]")
                })?;
                let msg = format!("installing skill {key}…");
                registry.lock().await.step(id, msg.clone());
                yield DownloadEvent {
                    event: Some(download_event::Event::Step(DownloadStep { id, message: msg })),
                };
                let cache_dest = cache_dir.join(key.as_str());
                git_sync(&skill.repo, &cache_dest)
                    .await
                    .with_context(|| format!("failed to sync skill repo for {key}"))?;

                let src = cache_dest.join(skill.path.as_str());
                let dst = skills_dir.join(key.as_str());
                if dst.exists() {
                    std::fs::remove_dir_all(&dst)
                        .with_context(|| format!("failed to remove old skill {key}"))?;
                }
                copy_dir_all(&src, &dst)
                    .with_context(|| format!("failed to copy skill {key}"))?;
            }
        }

        // Install agents (copy prompt .md files).
        if !manifest.agents.is_empty() {
            let msg = "installing agents…".to_string();
            registry.lock().await.step(id, msg.clone());
            yield DownloadEvent {
                event: Some(download_event::Event::Step(DownloadStep { id, message: msg })),
            };
            install_agents(scope, &manifest)?;
        }

        registry.lock().await.complete(id);
        yield DownloadEvent {
            event: Some(download_event::Event::Completed(DownloadCompleted { id })),
        };
    }
}

/// Uninstall a hub package, streaming unified download events.
///
/// Reads the manifest from the local hub repo (no network sync), removes MCP
/// servers and services from `walrus.toml`, deletes skill directories and
/// agent prompt files.
pub fn uninstall(
    package: CompactString,
    registry: std::sync::Arc<tokio::sync::Mutex<DownloadRegistry>>,
) -> impl futures_core::Stream<Item = Result<DownloadEvent>> {
    async_stream::try_stream! {
        let id = registry
            .lock()
            .await
            .start(DownloadKind::Hub, package.to_string());
        yield DownloadEvent {
            event: Some(download_event::Event::Created(DownloadCreated {
                id,
                kind: DownloadKind::Hub as i32,
                label: package.to_string(),
            })),
        };

        let (scope, name) = parse_package(&package)?;
        let manifest = read_manifest(scope, name)?;

        if !manifest.mcp_servers.is_empty() {
            let msg = "removing MCP servers…".to_string();
            registry.lock().await.step(id, msg.clone());
            yield DownloadEvent {
                event: Some(download_event::Event::Step(DownloadStep { id, message: msg })),
            };
            remove_mcp_servers(&manifest)?;
        }

        if !manifest.services.is_empty() {
            let msg = "removing services…".to_string();
            registry.lock().await.step(id, msg.clone());
            yield DownloadEvent {
                event: Some(download_event::Event::Step(DownloadStep { id, message: msg })),
            };
            remove_services(&manifest)?;
        }

        if !manifest.skills.is_empty() {
            let msg = "removing skills…".to_string();
            registry.lock().await.step(id, msg.clone());
            yield DownloadEvent {
                event: Some(download_event::Event::Step(DownloadStep { id, message: msg })),
            };
            let skills_dir = CONFIG_DIR.join("skills");
            for key in manifest.skills.keys() {
                let dst = skills_dir.join(key.as_str());
                if dst.exists() {
                    std::fs::remove_dir_all(&dst)
                        .with_context(|| format!("failed to remove skill {key}"))?;
                }
            }
        }

        if !manifest.agents.is_empty() {
            let msg = "removing agents…".to_string();
            registry.lock().await.step(id, msg.clone());
            yield DownloadEvent {
                event: Some(download_event::Event::Step(DownloadStep { id, message: msg })),
            };
            remove_agents(&manifest)?;
        }

        registry.lock().await.complete(id);
        yield DownloadEvent {
            event: Some(download_event::Event::Completed(DownloadCompleted { id })),
        };
    }
}

// ── Helpers ───────────────────────────────────────────────────────

/// Ensure `dest` is a shallow clone of `url`, creating or updating as needed.
async fn git_sync(url: &str, dest: &Path) -> Result<()> {
    if dest.exists() {
        let status = Command::new("git")
            .args([
                "-C",
                &dest.to_string_lossy(),
                "fetch",
                "--depth=1",
                "origin",
            ])
            .status()
            .await
            .context("git fetch failed")?;
        anyhow::ensure!(status.success(), "git fetch exited with {status}");

        let status = Command::new("git")
            .args([
                "-C",
                &dest.to_string_lossy(),
                "reset",
                "--hard",
                "origin/HEAD",
            ])
            .status()
            .await
            .context("git reset failed")?;
        anyhow::ensure!(status.success(), "git reset exited with {status}");
    } else {
        let status = Command::new("git")
            .args(["clone", "--depth=1", url, &dest.to_string_lossy()])
            .status()
            .await
            .context("git clone failed")?;
        anyhow::ensure!(status.success(), "git clone exited with {status}");
    }
    Ok(())
}

/// Parse a `scope/name` package string into `(scope, name)`.
fn parse_package(package: &str) -> Result<(&str, &str)> {
    let mut parts = package.splitn(2, '/');
    let scope = parts.next().filter(|s| !s.is_empty());
    let name = parts.next().filter(|s| !s.is_empty());
    match (scope, name) {
        (Some(s), Some(n)) => Ok((s, n)),
        _ => anyhow::bail!("package must be in `scope/name` format, got: {package}"),
    }
}

/// Read and deserialize the manifest for a package from the local hub repo.
fn read_manifest(scope: &str, name: &str) -> Result<manifest::Manifest> {
    let hub_dir = CONFIG_DIR.join("hub");
    let path = hub_dir.join(scope).join(format!("{name}.toml"));
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read manifest at {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("invalid manifest at {}", path.display()))
}

/// Merge MCP server entries from a manifest into `walrus.toml`.
fn merge_mcp_servers(manifest: &manifest::Manifest) -> Result<()> {
    use toml_edit::DocumentMut;

    let config_path = CONFIG_DIR.join("walrus.toml");
    let content = std::fs::read_to_string(&config_path)
        .with_context(|| format!("cannot read {}", config_path.display()))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("invalid TOML in {}", config_path.display()))?;

    let table = doc
        .entry("mcps")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .context("mcps is not a table")?;

    for (key, cfg) in &manifest.mcp_servers {
        let doc = toml_edit::ser::to_document(cfg)
            .with_context(|| format!("failed to serialize McpServerConfig for {key}"))?;
        let item = toml_edit::Item::Table(doc.as_table().clone());
        table.insert(key.as_str(), item);
    }

    std::fs::write(&config_path, doc.to_string())
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(())
}

/// Remove MCP server entries listed in a manifest from `walrus.toml`.
fn remove_mcp_servers(manifest: &manifest::Manifest) -> Result<()> {
    use toml_edit::DocumentMut;

    let config_path = CONFIG_DIR.join("walrus.toml");
    let content = std::fs::read_to_string(&config_path)
        .with_context(|| format!("cannot read {}", config_path.display()))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("invalid TOML in {}", config_path.display()))?;

    if let Some(table) = doc.get_mut("mcps").and_then(|v| v.as_table_mut()) {
        for key in manifest.mcp_servers.keys() {
            table.remove(key.as_str());
        }
    }

    std::fs::write(&config_path, doc.to_string())
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(())
}

/// Merge service entries from a manifest into `walrus.toml`.
///
/// Strips install-time fields (`install`, `description`) before writing —
/// they are hub metadata, not runtime config.
fn merge_services(manifest: &manifest::Manifest) -> Result<()> {
    use toml_edit::DocumentMut;

    let config_path = CONFIG_DIR.join("walrus.toml");
    let content = std::fs::read_to_string(&config_path)
        .with_context(|| format!("cannot read {}", config_path.display()))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("invalid TOML in {}", config_path.display()))?;

    let table = doc
        .entry("services")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .context("services is not a table")?;

    for (key, cfg) in &manifest.services {
        let mut runtime = cfg.clone();
        runtime.install = None;
        runtime.description = None;
        let doc = toml_edit::ser::to_document(&runtime)
            .with_context(|| format!("failed to serialize ServiceConfig for {key}"))?;
        let item = toml_edit::Item::Table(doc.as_table().clone());
        table.insert(key.as_str(), item);
    }

    std::fs::write(&config_path, doc.to_string())
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(())
}

/// Remove service entries listed in a manifest from `walrus.toml`.
fn remove_services(manifest: &manifest::Manifest) -> Result<()> {
    use toml_edit::DocumentMut;

    let config_path = CONFIG_DIR.join("walrus.toml");
    let content = std::fs::read_to_string(&config_path)
        .with_context(|| format!("cannot read {}", config_path.display()))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("invalid TOML in {}", config_path.display()))?;

    if let Some(table) = doc.get_mut("services").and_then(|v| v.as_table_mut()) {
        for key in manifest.services.keys() {
            table.remove(key.as_str());
        }
    }

    std::fs::write(&config_path, doc.to_string())
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(())
}

/// Install agent prompt files from the hub repo to `~/.openwalrus/agents/`.
fn install_agents(scope: &str, manifest: &manifest::Manifest) -> Result<()> {
    let agents_dir = CONFIG_DIR.join(wcore::paths::AGENTS_DIR);
    std::fs::create_dir_all(&agents_dir).context("failed to create agents directory")?;

    let hub_dir = CONFIG_DIR.join("hub");
    for (name, agent) in &manifest.agents {
        let src = hub_dir.join(scope).join(agent.prompt.as_str());
        let dst = agents_dir.join(format!("{name}.md"));
        std::fs::copy(&src, &dst).with_context(|| {
            format!(
                "failed to copy agent prompt {} -> {}",
                src.display(),
                dst.display()
            )
        })?;
    }
    Ok(())
}

/// Remove agent prompt files installed by a manifest.
fn remove_agents(manifest: &manifest::Manifest) -> Result<()> {
    let agents_dir = CONFIG_DIR.join(wcore::paths::AGENTS_DIR);
    for name in manifest.agents.keys() {
        let dst = agents_dir.join(format!("{name}.md"));
        if dst.exists() {
            std::fs::remove_file(&dst)
                .with_context(|| format!("failed to remove agent prompt {}", dst.display()))?;
        }
    }
    Ok(())
}

/// Recursively copy `src` directory into `dst`.
fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in
        std::fs::read_dir(src).with_context(|| format!("cannot read dir {}", src.display()))?
    {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)
                .with_context(|| format!("failed to copy {}", entry.path().display()))?;
        }
    }
    Ok(())
}
