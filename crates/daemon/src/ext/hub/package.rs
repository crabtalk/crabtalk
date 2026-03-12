//! Walrus hub package install/uninstall operations.

use crate::ext::hub::{DownloadRegistry, manifest};
use anyhow::{Context, Result};
use compact_str::CompactString;
use std::path::Path;
use tokio::process::Command;
use wcore::paths::CONFIG_DIR;
use wcore::protocol::message::server::{DownloadEvent, DownloadKind};

/// Remote URL of the walrus hub repository.
const WALRUS_HUB: &str = "https://github.com/openwalrus/hub";

/// Install a hub package, streaming unified download events.
///
/// Syncs the hub repo, reads the manifest, merges MCP servers into
/// `walrus.toml`, and copies skill directories into `~/.openwalrus/skills/`.
pub fn install(
    package: CompactString,
    registry: std::sync::Arc<tokio::sync::Mutex<DownloadRegistry>>,
) -> impl futures_core::Stream<Item = Result<DownloadEvent>> {
    async_stream::try_stream! {
        let id = registry
            .lock()
            .await
            .start(DownloadKind::Hub, package.to_string());
        yield DownloadEvent::Created {
            id,
            kind: DownloadKind::Hub,
            label: package.to_string(),
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
            yield DownloadEvent::Step { id, message: msg };
            merge_mcp_servers(&manifest)?;
        }

        // Install skills.
        if !manifest.skills.is_empty() {
            let msg = "installing skills…".to_string();
            registry.lock().await.step(id, msg.clone());
            yield DownloadEvent::Step { id, message: msg };
            let cache_dir = CONFIG_DIR.join(".cache").join("skills");
            let skills_dir = CONFIG_DIR.join("skills");
            std::fs::create_dir_all(&cache_dir).context("failed to create skill cache dir")?;
            std::fs::create_dir_all(&skills_dir).context("failed to create skills dir")?;

            for (key, skill) in &manifest.skills {
                let msg = format!("installing skill {key}…");
                registry.lock().await.step(id, msg.clone());
                yield DownloadEvent::Step { id, message: msg };
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

        registry.lock().await.complete(id);
        yield DownloadEvent::Completed { id };
    }
}

/// Uninstall a hub package, streaming unified download events.
///
/// Reads the manifest from the local hub repo (no network sync), removes MCP
/// server entries from `walrus.toml`, and deletes skill directories.
pub fn uninstall(
    package: CompactString,
    registry: std::sync::Arc<tokio::sync::Mutex<DownloadRegistry>>,
) -> impl futures_core::Stream<Item = Result<DownloadEvent>> {
    async_stream::try_stream! {
        let id = registry
            .lock()
            .await
            .start(DownloadKind::Hub, package.to_string());
        yield DownloadEvent::Created {
            id,
            kind: DownloadKind::Hub,
            label: package.to_string(),
        };

        let (scope, name) = parse_package(&package)?;
        let manifest = read_manifest(scope, name)?;

        if !manifest.mcp_servers.is_empty() {
            let msg = "removing MCP servers…".to_string();
            registry.lock().await.step(id, msg.clone());
            yield DownloadEvent::Step { id, message: msg };
            remove_mcp_servers(&manifest)?;
        }

        if !manifest.skills.is_empty() {
            let msg = "removing skills…".to_string();
            registry.lock().await.step(id, msg.clone());
            yield DownloadEvent::Step { id, message: msg };
            let skills_dir = CONFIG_DIR.join("skills");
            for key in manifest.skills.keys() {
                let dst = skills_dir.join(key.as_str());
                if dst.exists() {
                    std::fs::remove_dir_all(&dst)
                        .with_context(|| format!("failed to remove skill {key}"))?;
                }
            }
        }

        registry.lock().await.complete(id);
        yield DownloadEvent::Completed { id };
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
