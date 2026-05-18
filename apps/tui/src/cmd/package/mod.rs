//! Crabtalk package install/uninstall operations.

pub mod manifest;

use anyhow::{Context, Result};
use std::path::Path;
use wcore::paths::CONFIG_DIR;

const PACKAGES_REGISTRY: &str = "https://github.com/crabtalk/plugins";

pub async fn install(
    package: &str,
    branch: Option<&str>,
    path: Option<&Path>,
    force: bool,
    on_step: impl Fn(&str),
    on_output: impl Fn(&str),
) -> Result<()> {
    let name = validate_name(package)?;

    if !force {
        let manifest_path = CONFIG_DIR
            .join(wcore::paths::PACKAGES_DIR)
            .join(format!("{name}.toml"));
        if manifest_path.exists() {
            on_step("already installed, use --force to overwrite");
            return Ok(());
        }
    }

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

    let manifest = read_manifest_from(&registry_dir, name)?;
    let manifest_src = registry_dir.join(format!("{name}.toml"));

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

    if !manifest.commands.is_empty() {
        install_commands(&manifest, &on_step)?;
    }

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

pub async fn uninstall(package: &str, on_step: impl Fn(&str)) -> Result<()> {
    let name = validate_name(package)?;
    let manifest = read_manifest(name).ok();

    if let Some(ref manifest) = manifest
        && !manifest.commands.is_empty()
    {
        for (name, cmd) in &manifest.commands {
            on_step(&format!("uninstalling command {name} ({})…", cmd.krate));
            let _ = cargo_uninstall(&cmd.krate);
        }
    }

    on_step("removing manifest…");
    let manifest_path = CONFIG_DIR
        .join(wcore::paths::PACKAGES_DIR)
        .join(format!("{name}.toml"));
    if manifest_path.exists() {
        std::fs::remove_file(&manifest_path)
            .with_context(|| format!("failed to remove {}", manifest_path.display()))?;
    }

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

async fn git_sync(url: &str, dest: &Path, branch: Option<&str>) -> Result<()> {
    use tokio::process::Command;

    let dest_str = dest.to_string_lossy();

    if dest.exists() {
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

fn validate_name(package: &str) -> Result<&str> {
    let name = package.trim();
    anyhow::ensure!(!name.is_empty(), "package name cannot be empty");
    Ok(name)
}

fn install_commands(manifest: &manifest::Manifest, on_step: &impl Fn(&str)) -> Result<()> {
    for (name, cmd) in &manifest.commands {
        on_step(&format!("installing command {name} ({})…", cmd.krate));
        cargo_install(&cmd.krate)?;
    }
    Ok(())
}

fn cargo_install(krate: &str) -> Result<()> {
    let status = std::process::Command::new("cargo")
        .args(["install", krate])
        .status()
        .context("failed to run `cargo`")?;
    anyhow::ensure!(status.success(), "cargo install {krate} failed");
    Ok(())
}

fn cargo_uninstall(krate: &str) -> Result<()> {
    let status = std::process::Command::new("cargo")
        .args(["uninstall", krate])
        .status()
        .context("failed to run `cargo`")?;
    anyhow::ensure!(status.success(), "cargo uninstall {krate} failed");
    Ok(())
}

fn read_manifest(name: &str) -> Result<manifest::Manifest> {
    read_manifest_from(&CONFIG_DIR.join("registry"), name)
}

fn read_manifest_from(dir: &Path, name: &str) -> Result<manifest::Manifest> {
    let path = dir.join(format!("{name}.toml"));
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read manifest at {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("invalid manifest at {}", path.display()))
}
