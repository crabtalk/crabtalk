//! Crabtalk hub package install/uninstall operations.
//!
//! Install copies a manifest to `packages/scope/name.toml` and clones the
//! source repo to `.cache/repos/{slug}`. Skills and agents are discovered
//! from the cached repo by convention on daemon reload.

use crate::manifest;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use wcore::{Setup, paths::CONFIG_DIR};

/// Remote URL of the crabtalk hub repository.
pub const CRABTALK_HUB: &str = "https://github.com/crabtalk/hub";

/// Result of a successful install, carrying info the CLI needs for prompt setup.
pub struct InstallResult {
    /// Setup configuration from the manifest, if any.
    pub setup: Option<Setup>,
    /// Path to the cached source repo, if cloned.
    pub repo_dir: Option<PathBuf>,
}

/// Install a hub package.
///
/// Syncs the hub repo, copies the manifest to `packages/scope/name.toml`,
/// and clones the source repo to `.cache/repos/{slug}/`. Runs command-type
/// setup if configured. Returns [`InstallResult`] so the CLI can handle
/// prompt-type setup after daemon reload.
pub async fn install(
    package: &str,
    branch: Option<&str>,
    on_step: impl Fn(&str),
) -> Result<InstallResult> {
    let (scope, name) = parse_package(package)?;

    // Sync hub repo (clone or update).
    on_step("syncing hub…");
    let hub_dir = CONFIG_DIR.join("hub");
    git_sync(CRABTALK_HUB, &hub_dir, branch)
        .await
        .context("failed to sync hub repo")?;

    // Read the manifest from the hub repo.
    let manifest = read_manifest(scope, name)?;

    // Copy manifest to packages/scope/name.toml.
    on_step("installing manifest…");
    let packages_dir = CONFIG_DIR.join(wcore::paths::PACKAGES_DIR);
    let scope_dir = packages_dir.join(scope);
    std::fs::create_dir_all(&scope_dir)
        .with_context(|| format!("failed to create {}", scope_dir.display()))?;
    let manifest_dst = scope_dir.join(format!("{name}.toml"));
    let manifest_src = hub_dir.join(scope).join(format!("{name}.toml"));
    std::fs::copy(&manifest_src, &manifest_dst).with_context(|| {
        format!(
            "failed to copy manifest {} → {}",
            manifest_src.display(),
            manifest_dst.display()
        )
    })?;

    // Clone the source repo to .cache/repos/{slug}/ if it has a repository URL.
    let repo_dir = if !manifest.package.repository.is_empty() {
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

    // Run command-type setup from the cached repo.
    if let Some(Setup::Command { ref command }) = manifest.package.setup
        && let Some(ref dir) = repo_dir
    {
        on_step("running setup command…");
        let status = tokio::process::Command::new("sh")
            .args(["-c", command])
            .current_dir(dir)
            .status()
            .await
            .with_context(|| format!("failed to run setup command: {command}"))?;
        anyhow::ensure!(status.success(), "setup command exited with {status}");
    }

    Ok(InstallResult {
        setup: manifest.package.setup,
        repo_dir,
    })
}

/// Uninstall a hub package.
///
/// Deletes the manifest from `packages/scope/name.toml` and optionally
/// prunes the cached source repo.
pub async fn uninstall(package: &str, on_step: impl Fn(&str)) -> Result<()> {
    let (scope, name) = parse_package(package)?;

    // Read manifest before deleting (need repository URL for cache cleanup).
    let manifest = read_manifest(scope, name).ok();

    // Delete manifest from packages/.
    on_step("removing manifest…");
    let manifest_path = CONFIG_DIR
        .join(wcore::paths::PACKAGES_DIR)
        .join(scope)
        .join(format!("{name}.toml"));
    if manifest_path.exists() {
        std::fs::remove_file(&manifest_path)
            .with_context(|| format!("failed to remove {}", manifest_path.display()))?;
    }

    // Clean up empty scope directory.
    let scope_dir = CONFIG_DIR.join(wcore::paths::PACKAGES_DIR).join(scope);
    if scope_dir.exists()
        && std::fs::read_dir(&scope_dir)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false)
    {
        let _ = std::fs::remove_dir(&scope_dir);
    }

    // Prune cached repo if no other package references it.
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

/// Parse a `scope/name` package string into `(scope, name)`.
pub fn parse_package(package: &str) -> Result<(&str, &str)> {
    let mut parts = package.splitn(2, '/');
    let scope = parts.next().filter(|s| !s.is_empty());
    let name = parts.next().filter(|s| !s.is_empty());
    match (scope, name) {
        (Some(s), Some(n)) => Ok((s, n)),
        _ => anyhow::bail!("package must be in `scope/name` format, got: {package}"),
    }
}

/// Read and deserialize the manifest for a package from the local hub repo.
pub fn read_manifest(scope: &str, name: &str) -> Result<manifest::Manifest> {
    let hub_dir = CONFIG_DIR.join("hub");
    let path = hub_dir.join(scope).join(format!("{name}.toml"));
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read manifest at {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("invalid manifest at {}", path.display()))
}
