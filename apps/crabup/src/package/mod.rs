//! Crabtalk package install/uninstall operations.
//!
//! Install copies a manifest to `packages/name.toml` and clones the
//! source repo to `.cache/repos/{slug}`. Skills and agents are discovered
//! from the cached repo by convention on daemon reload.

pub mod manifest;

use anyhow::{Context, Result};
use std::path::Path;
use wcore::paths::CONFIG_DIR;

/// Remote URL of the crabtalk packages registry.
pub const PACKAGES_REGISTRY: &str = "https://github.com/crabtalk/plugins";

/// Install a package.
///
/// Syncs the package registry, copies the manifest to `packages/name.toml`,
/// and clones the source repo to `.cache/repos/{slug}/`. Runs setup
/// script if configured.
pub async fn install(
    package: &str,
    branch: Option<&str>,
    path: Option<&Path>,
    force: bool,
    on_step: impl Fn(&str),
    on_output: impl Fn(&str),
) -> Result<()> {
    let name = validate_name(package)?;

    // Check if already installed.
    if !force {
        let manifest_path = CONFIG_DIR
            .join(wcore::paths::PACKAGES_DIR)
            .join(format!("{name}.toml"));
        if manifest_path.exists() {
            on_step("already installed, use --force to overwrite");
            return Ok(());
        }
    }

    // Resolve the registry directory — use a local path or sync from remote.
    let registry_dir = if let Some(p) = path {
        anyhow::ensure!(p.exists(), "package path {} does not exist", p.display());
        p.to_path_buf()
    } else {
        on_step("syncing package registry…");
        let dir = CONFIG_DIR.join("registry");
        git_sync(PACKAGES_REGISTRY, &dir, branch)
            .await
            .context("failed to sync package registry")?;
        dir
    };

    // Read the manifest from the registry directory.
    let manifest = read_manifest_from(&registry_dir, name)?;
    let manifest_src = registry_dir.join(format!("{name}.toml"));

    // Clone the source repo only when the package has resources that live
    // inside it: a setup script or agent files. MCPs connect directly and
    // commands are installed via `cargo install` — neither needs the repo.
    let needs_repo = manifest.package.setup.is_some() || !manifest.agents.is_empty();
    let repo_dir = if !manifest.package.repository.is_empty() && needs_repo {
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
        install_commands(&manifest, &on_step)?;
    }

    // Copy manifest to packages/name.toml — done last so a failed
    // setup doesn't leave a half-installed package that blocks re-install.
    on_step("installing manifest…");
    let packages_dir = CONFIG_DIR.join(wcore::paths::PACKAGES_DIR);
    std::fs::create_dir_all(&packages_dir)
        .with_context(|| format!("failed to create {}", packages_dir.display()))?;
    let manifest_dst = packages_dir.join(format!("{name}.toml"));
    std::fs::copy(&manifest_src, &manifest_dst).with_context(|| {
        format!(
            "failed to copy manifest {} → {}",
            manifest_src.display(),
            manifest_dst.display()
        )
    })?;

    Ok(())
}

/// Uninstall a package.
///
/// Deletes the manifest from `packages/name.toml` and optionally
/// prunes the cached source repo.
pub async fn uninstall(package: &str, on_step: impl Fn(&str)) -> Result<()> {
    let name = validate_name(package)?;

    // Read manifest before deleting (need repository URL for cache cleanup).
    let manifest = read_manifest(name).ok();

    // Uninstall command crates (best-effort — don't fail if already removed).
    if let Some(ref manifest) = manifest
        && !manifest.commands.is_empty()
    {
        for (name, cmd) in &manifest.commands {
            on_step(&format!("uninstalling command {name} ({})…", cmd.krate));
            let _ = crate::cargo::uninstall(&cmd.krate);
        }
    }

    // Delete manifest from packages/.
    on_step("removing manifest…");
    let manifest_path = CONFIG_DIR
        .join(wcore::paths::PACKAGES_DIR)
        .join(format!("{name}.toml"));
    if manifest_path.exists() {
        std::fs::remove_file(&manifest_path)
            .with_context(|| format!("failed to remove {}", manifest_path.display()))?;
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

/// Validate a package name is non-empty.
fn validate_name(package: &str) -> Result<&str> {
    let name = package.trim();
    anyhow::ensure!(!name.is_empty(), "package name cannot be empty");
    Ok(name)
}

/// Install command crates from a manifest via `cargo install`.
fn install_commands(manifest: &manifest::Manifest, on_step: &impl Fn(&str)) -> Result<()> {
    for (name, cmd) in &manifest.commands {
        on_step(&format!("installing command {name} ({})…", cmd.krate));
        crate::cargo::install(&cmd.krate, Default::default())?;
    }
    Ok(())
}

/// Read and deserialize a manifest from the default package registry directory.
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
