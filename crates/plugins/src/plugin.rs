//! Crabtalk plugin install/uninstall operations.
//!
//! Install copies a manifest to `plugins/name.toml` and clones the
//! source repo to `.cache/repos/{slug}`. Skills and agents are discovered
//! from the cached repo by convention on daemon reload.

use crate::manifest;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use wcore::paths::CONFIG_DIR;

/// Remote URL of the crabtalk plugins registry.
pub const PLUGINS_REGISTRY: &str = "https://github.com/crabtalk/plugins";

/// Install a plugin.
///
/// Syncs the plugin registry, copies the manifest to `plugins/name.toml`,
/// and clones the source repo to `.cache/repos/{slug}/`. Runs setup
/// script if configured.
pub async fn install(
    plugin: &str,
    branch: Option<&str>,
    path: Option<&Path>,
    force: bool,
    on_step: impl Fn(&str),
    on_output: impl Fn(&str),
) -> Result<()> {
    let name = validate_name(plugin)?;

    // Check if already installed.
    if !force {
        let manifest_path = CONFIG_DIR
            .join(wcore::paths::PLUGINS_DIR)
            .join(format!("{name}.toml"));
        if manifest_path.exists() {
            on_step("already installed, use --force to overwrite");
            return Ok(());
        }
    }

    // Resolve the registry directory — use a local path or sync from remote.
    let registry_dir = if let Some(p) = path {
        anyhow::ensure!(p.exists(), "plugin path {} does not exist", p.display());
        p.to_path_buf()
    } else {
        on_step("syncing plugin registry…");
        let dir = CONFIG_DIR.join("registry");
        git_sync(PLUGINS_REGISTRY, &dir, branch)
            .await
            .context("failed to sync plugin registry")?;
        dir
    };

    // Read the manifest from the registry directory.
    let manifest = read_manifest_from(&registry_dir, name)?;
    let manifest_src = registry_dir.join(format!("{name}.toml"));

    // Clone the source repo if the plugin has resources that live in
    // the repo (setup scripts, agents, or skills). Plugins that only
    // declare MCPs or commands don't need the repo — MCPs connect
    // directly and commands are installed via `cargo install`.
    let mcp_only =
        manifest.package.setup.is_none() && manifest.agents.is_empty() && !manifest.mcps.is_empty();
    let repo_dir = if !manifest.package.repository.is_empty() && !mcp_only {
        on_step("cloning source repo…");
        let slug = wcore::repo_slug(&manifest.package.repository);
        let dir = CONFIG_DIR.join(".cache").join("repos").join(&slug);
        std::fs::create_dir_all(dir.parent().context("repo cache path has no parent")?)
            .context("failed to create repo cache dir")?;
        let effective_branch = manifest.package.branch.as_deref();
        git_sync(&manifest.package.repository, &dir, effective_branch)
            .await
            .with_context(|| format!("failed to sync repo {}", &manifest.package.repository))?;
        Some(dir)
    } else {
        None
    };

    // Run setup script from the cached repo, streaming output line by line.
    if let Some(ref setup) = manifest.package.setup
        && let Some(ref dir) = repo_dir
    {
        use tokio::io::{AsyncBufReadExt, BufReader};
        use tokio::process::Command;

        let script = &setup.script;
        on_step("running setup script…");
        let is_file = !script.contains(' ') && dir.join(script).is_file();
        let mut child = if is_file {
            Command::new("bash")
                .arg(script)
                .current_dir(dir)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
        } else {
            Command::new("bash")
                .args(["-c", script])
                .current_dir(dir)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
        }
        .with_context(|| format!("failed to spawn setup script: {script}"))?;

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let mut stdout_lines = BufReader::new(stdout).lines();
        let mut stderr_lines = BufReader::new(stderr).lines();

        loop {
            tokio::select! {
                line = stdout_lines.next_line() => match line {
                    Ok(Some(line)) => on_output(&line),
                    Ok(None) => break,
                    Err(_) => break,
                },
                line = stderr_lines.next_line() => match line {
                    Ok(Some(line)) => on_output(&line),
                    Ok(None) => {}
                    Err(_) => {}
                },
            }
        }

        let status = child
            .wait()
            .await
            .with_context(|| format!("failed to wait for setup script: {script}"))?;
        anyhow::ensure!(status.success(), "setup script exited with {status}");
    }

    // Auto-install command crates via `cargo install`.
    if !manifest.commands.is_empty() {
        install_commands(&manifest, &on_step).await?;
    }

    // Copy manifest to plugins/name.toml — done last so a failed
    // setup doesn't leave a half-installed plugin that blocks re-install.
    on_step("installing manifest…");
    let plugins_dir = CONFIG_DIR.join(wcore::paths::PLUGINS_DIR);
    std::fs::create_dir_all(&plugins_dir)
        .with_context(|| format!("failed to create {}", plugins_dir.display()))?;
    let manifest_dst = plugins_dir.join(format!("{name}.toml"));
    std::fs::copy(&manifest_src, &manifest_dst).with_context(|| {
        format!(
            "failed to copy manifest {} → {}",
            manifest_src.display(),
            manifest_dst.display()
        )
    })?;

    Ok(())
}

/// Uninstall a plugin.
///
/// Deletes the manifest from `plugins/name.toml` and optionally
/// prunes the cached source repo.
pub async fn uninstall(plugin: &str, on_step: impl Fn(&str)) -> Result<()> {
    let name = validate_name(plugin)?;

    // Read manifest before deleting (need repository URL for cache cleanup).
    let manifest = read_manifest(name).ok();

    // Uninstall command crates (best-effort — don't fail if already removed).
    if let Some(ref manifest) = manifest
        && !manifest.commands.is_empty()
        && let Ok(cargo) = find_cargo(&on_step).await
    {
        for (name, cmd) in &manifest.commands {
            on_step(&format!("uninstalling command {name} ({})…", cmd.krate));
            let _ = tokio::process::Command::new(&cargo)
                .args(["uninstall", &cmd.krate])
                .status()
                .await;
        }
    }

    // Delete manifest from plugins/.
    on_step("removing manifest…");
    let manifest_path = CONFIG_DIR
        .join(wcore::paths::PLUGINS_DIR)
        .join(format!("{name}.toml"));
    if manifest_path.exists() {
        std::fs::remove_file(&manifest_path)
            .with_context(|| format!("failed to remove {}", manifest_path.display()))?;
    }

    // Prune cached repo if no other plugin references it.
    if let Some(manifest) = manifest
        && !manifest.package.repository.is_empty()
    {
        let slug = wcore::repo_slug(&manifest.package.repository);
        let repo_dir = CONFIG_DIR.join(".cache").join("repos").join(&slug);
        if repo_dir.exists() {
            on_step("pruning cached repo…");
            let _ = std::fs::remove_dir_all(&repo_dir);
        }
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────

/// Ensure `dest` is a shallow clone of `url`, creating or updating as needed.
/// If `branch` is provided, clone/fetch that specific branch.
pub async fn git_sync(url: &str, dest: &Path, branch: Option<&str>) -> Result<()> {
    use tokio::process::Command;

    let dest_str = dest.to_string_lossy();

    if dest.exists() {
        // Use an explicit refspec so git creates a proper remote tracking ref.
        // Plain `git fetch origin <branch>` only updates FETCH_HEAD which goes
        // stale across calls with different branches.
        let (refspec, ref_name) = match branch {
            Some(b) => (
                format!("+refs/heads/{b}:refs/remotes/origin/{b}"),
                format!("origin/{b}"),
            ),
            None => (String::new(), "origin/HEAD".to_string()),
        };
        let mut args = vec!["-C", &*dest_str, "fetch", "--depth=1", "origin"];
        if !refspec.is_empty() {
            args.push(&refspec);
        }
        let status = Command::new("git")
            .args(&args)
            .status()
            .await
            .context("git fetch failed")?;
        anyhow::ensure!(status.success(), "git fetch exited with {status}");

        let status = Command::new("git")
            .args(["-C", &*dest_str, "reset", "--hard", &ref_name])
            .status()
            .await
            .context("git reset failed")?;
        anyhow::ensure!(status.success(), "git reset exited with {status}");
    } else {
        let mut args = vec!["clone", "--depth=1"];
        if let Some(b) = branch {
            args.extend(["-b", b]);
        }
        args.extend([url, &*dest_str]);
        let status = Command::new("git")
            .args(&args)
            .status()
            .await
            .context("git clone failed")?;
        anyhow::ensure!(status.success(), "git clone exited with {status}");
    }
    Ok(())
}

/// Info about a plugin returned by [`search`].
pub struct PluginEntry {
    /// Plugin name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Number of skills in the plugin.
    pub skill_count: u32,
    /// Number of MCP servers in the plugin.
    pub mcp_count: u32,
    /// Whether the plugin is installed locally.
    pub installed: bool,
    /// Source repository URL.
    pub repository: String,
}

/// Search the plugin registry for plugins matching the query.
///
/// Syncs the registry repo, scans all `.toml` manifests, and returns
/// matching plugins. An empty query returns all plugins.
pub async fn search(query: &str) -> Result<Vec<PluginEntry>> {
    let registry_dir = CONFIG_DIR.join("registry");
    git_sync(PLUGINS_REGISTRY, &registry_dir, None)
        .await
        .context("failed to sync plugin registry")?;

    let plugins_dir = CONFIG_DIR.join(wcore::paths::PLUGINS_DIR);
    let query_lower = query.to_lowercase();
    let mut results = Vec::new();

    // Scan .toml manifests in the registry.
    let Ok(entries) = std::fs::read_dir(&registry_dir) else {
        return Ok(results);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(manifest) = toml::from_str::<manifest::Manifest>(&content) else {
            continue;
        };

        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();

        // Filter by query (match name, description, or keywords).
        if !query_lower.is_empty() {
            let matches = name.to_lowercase().contains(&query_lower)
                || manifest
                    .package
                    .description
                    .to_lowercase()
                    .contains(&query_lower)
                || manifest
                    .package
                    .keywords
                    .iter()
                    .any(|k| k.to_lowercase().contains(&query_lower));
            if !matches {
                continue;
            }
        }

        let installed = plugins_dir.join(format!("{name}.toml")).exists();
        let mcp_count = manifest.mcps.len() as u32;
        results.push(PluginEntry {
            name,
            description: manifest.package.description,
            skill_count: 0,
            mcp_count,
            installed,
            repository: manifest.package.repository,
        });
    }

    results.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(results)
}

/// Validate a plugin name is non-empty.
fn validate_name(plugin: &str) -> Result<&str> {
    let name = plugin.trim();
    anyhow::ensure!(!name.is_empty(), "plugin name cannot be empty");
    Ok(name)
}

/// Install command crates from a manifest via `cargo install`.
async fn install_commands(manifest: &manifest::Manifest, on_step: &impl Fn(&str)) -> Result<()> {
    let cargo = find_cargo(on_step).await?;
    for (name, cmd) in &manifest.commands {
        on_step(&format!("installing command {name} ({})…", cmd.krate));
        let status = tokio::process::Command::new(&cargo)
            .args(["install", &cmd.krate])
            .status()
            .await
            .with_context(|| format!("failed to run cargo install {}", cmd.krate))?;
        anyhow::ensure!(
            status.success(),
            "cargo install {} failed with {status}",
            cmd.krate
        );
    }
    Ok(())
}

/// Locate cargo, installing rustup if needed. Returns the path to the cargo binary.
async fn find_cargo(on_step: &impl Fn(&str)) -> Result<PathBuf> {
    if let Some(cargo) = which("cargo") {
        return Ok(cargo);
    }

    on_step("installing Rust via rustup…");
    let status = tokio::process::Command::new("sh")
        .args([
            "-c",
            "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
        ])
        .status()
        .await
        .context("failed to run rustup installer")?;
    anyhow::ensure!(status.success(), "rustup installation failed");

    let home = std::env::var("HOME").unwrap_or_default();
    let cargo = PathBuf::from(&home).join(".cargo/bin/cargo");
    anyhow::ensure!(
        cargo.exists(),
        "cargo not found at {} after rustup install",
        cargo.display()
    );
    Ok(cargo)
}

/// Look for a binary on PATH, falling back to `~/.cargo/bin`.
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var("PATH").unwrap_or_default();
    for dir in path.split(':') {
        let candidate = PathBuf::from(dir).join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let candidate = PathBuf::from(home).join(".cargo/bin").join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

/// Read and deserialize a manifest from the default plugin registry directory.
pub fn read_manifest(name: &str) -> Result<manifest::Manifest> {
    read_manifest_from(&CONFIG_DIR.join("registry"), name)
}

/// Read and deserialize a manifest from a given directory.
pub fn read_manifest_from(dir: &Path, name: &str) -> Result<manifest::Manifest> {
    let path = dir.join(format!("{name}.toml"));
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read manifest at {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("invalid manifest at {}", path.display()))
}
