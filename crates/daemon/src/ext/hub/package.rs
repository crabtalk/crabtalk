//! Walrus hub package install/uninstall operations.

use crate::ext::hub::{DownloadRegistry, manifest};
use anyhow::{Context, Result};
use compact_str::CompactString;
use std::collections::BTreeSet;
use std::path::Path;
use tokio::process::Command;
use wcore::paths::CONFIG_DIR;
use wcore::protocol::message::{
    DownloadCompleted, DownloadCreated, DownloadEvent, DownloadKind, DownloadStep, download_event,
};

/// Remote URL of the walrus hub repository.
const WALRUS_HUB: &str = "https://github.com/openwalrus/hub";

/// Parsed filters for selective install/uninstall.
///
/// When all sets are empty, everything is included (no filtering).
/// Filter format: `"kind:name"` — e.g. `"skill:playwright-cli"`.
struct HubFilter {
    mcps: BTreeSet<String>,
    services: BTreeSet<String>,
    skills: BTreeSet<String>,
    agents: BTreeSet<String>,
}

impl HubFilter {
    fn parse(filters: &[String]) -> Self {
        let mut f = Self {
            mcps: BTreeSet::new(),
            services: BTreeSet::new(),
            skills: BTreeSet::new(),
            agents: BTreeSet::new(),
        };
        for raw in filters {
            if let Some((kind, name)) = raw.split_once(':') {
                let set = match kind {
                    "mcp" => &mut f.mcps,
                    "service" => &mut f.services,
                    "skill" => &mut f.skills,
                    "agent" => &mut f.agents,
                    _ => continue,
                };
                set.insert(name.to_string());
            }
        }
        f
    }

    fn is_empty(&self) -> bool {
        self.mcps.is_empty()
            && self.services.is_empty()
            && self.skills.is_empty()
            && self.agents.is_empty()
    }

    fn wants_mcp(&self, name: &str) -> bool {
        self.is_empty() || self.mcps.contains(name)
    }

    fn wants_service(&self, name: &str) -> bool {
        self.is_empty() || self.services.contains(name)
    }

    fn wants_skill(&self, name: &str) -> bool {
        self.is_empty() || self.skills.contains(name)
    }

    fn wants_agent(&self, name: &str) -> bool {
        self.is_empty() || self.agents.contains(name)
    }
}

/// Install a hub package, streaming unified download events.
///
/// Syncs the hub repo, reads the manifest, merges MCP servers and services
/// into `walrus.toml`, copies skill directories into `~/.openwalrus/skills/`,
/// and installs agents (prompt files + their declared skills).
///
/// When `filters` is non-empty, only matching components are installed.
pub fn install(
    package: CompactString,
    registry: std::sync::Arc<tokio::sync::Mutex<DownloadRegistry>>,
    filters: Vec<String>,
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

        let filter = HubFilter::parse(&filters);

        // Sync hub repo (clone or update).
        let hub_dir = CONFIG_DIR.join("hub");
        git_sync(WALRUS_HUB, &hub_dir).await.context("failed to sync hub repo")?;

        let (scope, name) = parse_package(&package)?;
        let manifest = read_manifest(scope, name)?;

        // Merge MCP servers.
        let wanted_mcps: Vec<_> = manifest
            .mcp_servers
            .iter()
            .filter(|(k, _)| filter.wants_mcp(k.as_str()))
            .collect();
        if !wanted_mcps.is_empty() {
            let msg = "adding MCP servers…".to_string();
            registry.lock().await.step(id, msg.clone());
            yield DownloadEvent {
                event: Some(download_event::Event::Step(DownloadStep { id, message: msg })),
            };
            merge_mcp_servers_filtered(&wanted_mcps)?;
        }

        // Install service binaries and merge configs.
        let wanted_services: Vec<_> = manifest
            .services
            .iter()
            .filter(|(k, _)| filter.wants_service(k.as_str()))
            .collect();
        if !wanted_services.is_empty() {
            // Auto-install Rust toolchain if cargo is not available.
            // Check the well-known path first since the daemon's PATH may not
            // include ~/.cargo/bin.
            let cargo = std::env::var_os("HOME")
                .map(|h| std::path::PathBuf::from(h).join(".cargo/bin/cargo"))
                .filter(|p| p.exists());
            let cargo_bin = cargo.as_deref().unwrap_or(Path::new("cargo"));
            if Command::new(cargo_bin).arg("--version").output().await.is_err() {
                let msg = "installing rust toolchain…".to_string();
                registry.lock().await.step(id, msg.clone());
                yield DownloadEvent {
                    event: Some(download_event::Event::Step(DownloadStep { id, message: msg })),
                };
                let status = Command::new("sh")
                    .args(["-c", "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"])
                    .status()
                    .await
                    .context("failed to install rust via rustup")?;
                if !status.success() {
                    Err(anyhow::anyhow!("rustup install exited with {status}"))?;
                }
            }

            for (key, cfg) in &wanted_services {
                let msg = format!("installing {key}…");
                registry.lock().await.step(id, msg.clone());
                yield DownloadEvent {
                    event: Some(download_event::Event::Step(DownloadStep { id, message: msg })),
                };
                let status = Command::new(cargo_bin)
                    .args(["install", &cfg.krate])
                    .status()
                    .await
                    .with_context(|| format!("failed to cargo install {key}"))?;
                if !status.success() {
                    Err(anyhow::anyhow!("cargo install for {key} exited with {status}"))?;
                }
            }

            let msg = "adding services…".to_string();
            registry.lock().await.step(id, msg.clone());
            yield DownloadEvent {
                event: Some(download_event::Event::Step(DownloadStep { id, message: msg })),
            };
            merge_services_filtered(&wanted_services)?;
        }

        // Collect skill keys from wanted agents.
        let wanted_agents: Vec<_> = manifest
            .agents
            .iter()
            .filter(|(k, _)| filter.wants_agent(k.as_str()))
            .collect();
        let mut agent_skill_keys: BTreeSet<&CompactString> = BTreeSet::new();
        for (_, agent) in &wanted_agents {
            for sk in &agent.skills {
                agent_skill_keys.insert(sk);
            }
        }

        // Install skills (top-level wanted + agent-referenced).
        let all_skill_keys: BTreeSet<&CompactString> = manifest
            .skills
            .keys()
            .filter(|k| filter.wants_skill(k.as_str()))
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
                git_sync(&manifest.package.repository, &cache_dest)
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
        if !wanted_agents.is_empty() {
            let msg = "installing agents…".to_string();
            registry.lock().await.step(id, msg.clone());
            yield DownloadEvent {
                event: Some(download_event::Event::Step(DownloadStep { id, message: msg })),
            };
            install_agents_filtered(scope, &wanted_agents)?;
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
///
/// When `filters` is non-empty, only matching components are removed.
pub fn uninstall(
    package: CompactString,
    registry: std::sync::Arc<tokio::sync::Mutex<DownloadRegistry>>,
    filters: Vec<String>,
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

        let filter = HubFilter::parse(&filters);

        let (scope, name) = parse_package(&package)?;
        let manifest = read_manifest(scope, name)?;

        let mcp_keys: Vec<_> = manifest
            .mcp_servers
            .keys()
            .filter(|k| filter.wants_mcp(k.as_str()))
            .collect();
        if !mcp_keys.is_empty() {
            let msg = "removing MCP servers…".to_string();
            registry.lock().await.step(id, msg.clone());
            yield DownloadEvent {
                event: Some(download_event::Event::Step(DownloadStep { id, message: msg })),
            };
            remove_keys_from_section("mcps", &mcp_keys)?;
        }

        let service_keys: Vec<_> = manifest
            .services
            .keys()
            .filter(|k| filter.wants_service(k.as_str()))
            .collect();
        if !service_keys.is_empty() {
            let msg = "removing services…".to_string();
            registry.lock().await.step(id, msg.clone());
            yield DownloadEvent {
                event: Some(download_event::Event::Step(DownloadStep { id, message: msg })),
            };
            remove_keys_from_section("services", &service_keys)?;
        }

        let skill_keys: Vec<_> = manifest
            .skills
            .keys()
            .filter(|k| filter.wants_skill(k.as_str()))
            .collect();
        if !skill_keys.is_empty() {
            let msg = "removing skills…".to_string();
            registry.lock().await.step(id, msg.clone());
            yield DownloadEvent {
                event: Some(download_event::Event::Step(DownloadStep { id, message: msg })),
            };
            let skills_dir = CONFIG_DIR.join("skills");
            for key in &skill_keys {
                let dst = skills_dir.join(key.as_str());
                if dst.exists() {
                    std::fs::remove_dir_all(&dst)
                        .with_context(|| format!("failed to remove skill {key}"))?;
                }
            }
        }

        let agent_keys: Vec<_> = manifest
            .agents
            .keys()
            .filter(|k| filter.wants_agent(k.as_str()))
            .collect();
        if !agent_keys.is_empty() {
            let msg = "removing agents…".to_string();
            registry.lock().await.step(id, msg.clone());
            yield DownloadEvent {
                event: Some(download_event::Event::Step(DownloadStep { id, message: msg })),
            };
            let agents_dir = CONFIG_DIR.join(wcore::paths::AGENTS_DIR);
            for name in &agent_keys {
                let dst = agents_dir.join(format!("{name}.md"));
                if dst.exists() {
                    std::fs::remove_file(&dst)
                        .with_context(|| format!("failed to remove agent prompt {}", dst.display()))?;
                }
            }
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

/// Merge selected MCP server entries into `walrus.toml`.
fn merge_mcp_servers_filtered(
    entries: &[(&CompactString, &wcore::config::mcp::McpServerConfig)],
) -> Result<()> {
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

    for (key, cfg) in entries {
        let doc = toml_edit::ser::to_document(cfg)
            .with_context(|| format!("failed to serialize McpServerConfig for {key}"))?;
        let item = toml_edit::Item::Table(doc.as_table().clone());
        table.insert(key.as_str(), item);
    }

    std::fs::write(&config_path, doc.to_string())
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(())
}

/// Merge selected service entries into `walrus.toml`.
fn merge_services_filtered(
    entries: &[(&CompactString, &crate::service::config::ServiceConfig)],
) -> Result<()> {
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

    for (key, cfg) in entries {
        let mut runtime = (*cfg).clone();
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

/// Remove keys from a TOML section in `walrus.toml`.
fn remove_keys_from_section(section: &str, keys: &[&CompactString]) -> Result<()> {
    use toml_edit::DocumentMut;

    let config_path = CONFIG_DIR.join("walrus.toml");
    let content = std::fs::read_to_string(&config_path)
        .with_context(|| format!("cannot read {}", config_path.display()))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("invalid TOML in {}", config_path.display()))?;

    if let Some(table) = doc.get_mut(section).and_then(|v| v.as_table_mut()) {
        for key in keys {
            table.remove(key.as_str());
        }
    }

    std::fs::write(&config_path, doc.to_string())
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(())
}

/// Install selected agent prompt files from the hub repo.
fn install_agents_filtered(
    scope: &str,
    agents: &[(&CompactString, &manifest::AgentResource)],
) -> Result<()> {
    let agents_dir = CONFIG_DIR.join(wcore::paths::AGENTS_DIR);
    std::fs::create_dir_all(&agents_dir).context("failed to create agents directory")?;

    let hub_dir = CONFIG_DIR.join("hub");
    for (name, agent) in agents {
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
