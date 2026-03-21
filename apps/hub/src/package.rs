//! Crabtalk hub package install/uninstall operations.

use crate::manifest;
use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::path::Path;
use wcore::paths::CONFIG_DIR;

/// Remote URL of the crabtalk hub repository.
pub const CRABTALK_HUB: &str = "https://github.com/crabtalk/hub";

/// Parsed filters for selective install/uninstall.
///
/// When all sets are empty, everything is included (no filtering).
/// Filter format: `"kind:name"` — e.g. `"skill:playwright-cli"`.
pub struct HubFilter {
    pub mcps: BTreeSet<String>,
    pub skills: BTreeSet<String>,
    pub agents: BTreeSet<String>,
    pub commands: BTreeSet<String>,
}

impl HubFilter {
    pub fn parse(filters: &[String]) -> Self {
        let mut f = Self {
            mcps: BTreeSet::new(),
            skills: BTreeSet::new(),
            agents: BTreeSet::new(),
            commands: BTreeSet::new(),
        };
        for raw in filters {
            if let Some((kind, name)) = raw.split_once(':') {
                let set = match kind {
                    "mcp" => &mut f.mcps,
                    "skill" => &mut f.skills,
                    "agent" => &mut f.agents,
                    "command" => &mut f.commands,
                    _ => continue,
                };
                set.insert(name.to_string());
            }
        }
        f
    }

    pub fn is_empty(&self) -> bool {
        self.mcps.is_empty()
            && self.skills.is_empty()
            && self.agents.is_empty()
            && self.commands.is_empty()
    }

    pub fn wants_mcp(&self, name: &str) -> bool {
        self.is_empty() || self.mcps.contains(name)
    }

    pub fn wants_skill(&self, name: &str) -> bool {
        self.is_empty() || self.skills.contains(name)
    }

    pub fn wants_agent(&self, name: &str) -> bool {
        self.is_empty() || self.agents.contains(name)
    }

    pub fn wants_command(&self, name: &str) -> bool {
        self.is_empty() || self.commands.contains(name)
    }
}

/// Install a hub package.
///
/// Syncs the hub repo, reads the manifest, merges MCP servers
/// into `config.toml`, copies skill directories into `~/.crabtalk/skills/`,
/// and installs agents (prompt files + their declared skills).
///
/// When `filters` is non-empty, only matching components are installed.
/// Progress messages are reported via `on_step`.
pub async fn install(package: &str, filters: &[String], on_step: impl Fn(&str)) -> Result<()> {
    let filter = HubFilter::parse(filters);

    // Sync hub repo (clone or update).
    let hub_dir = CONFIG_DIR.join("hub");
    git_sync(CRABTALK_HUB, &hub_dir)
        .await
        .context("failed to sync hub repo")?;

    let (scope, name) = parse_package(package)?;
    let manifest = read_manifest(scope, name)?;

    // Merge MCP servers (convert McpResource → McpServerConfig for config.toml).
    let wanted_mcps: Vec<_> = manifest
        .mcps
        .iter()
        .filter(|(k, _)| filter.wants_mcp(k.as_str()))
        .map(|(k, v)| (k, v.to_server_config()))
        .collect();
    if !wanted_mcps.is_empty() {
        on_step("adding MCP servers…");
        let refs: Vec<_> = wanted_mcps.iter().map(|(k, v)| (*k, v)).collect();
        merge_section_filtered("mcps", &refs)?;
    }

    // Collect skill keys from wanted agents.
    let wanted_agents: Vec<_> = manifest
        .agents
        .iter()
        .filter(|(k, _)| filter.wants_agent(k.as_str()))
        .collect();
    let mut agent_skill_keys: BTreeSet<&String> = BTreeSet::new();
    for (_, agent) in &wanted_agents {
        for sk in &agent.skills {
            agent_skill_keys.insert(sk);
        }
    }

    // Install skills (top-level wanted + agent-referenced).
    let all_skill_keys: BTreeSet<&String> = manifest
        .skills
        .keys()
        .filter(|k| filter.wants_skill(k.as_str()))
        .chain(agent_skill_keys.iter().copied())
        .collect();

    if !all_skill_keys.is_empty() {
        on_step("installing skills…");

        // Clone the source repo once, then copy per-skill subdirectories.
        let slug = repo_slug(&manifest.package.repository);
        if slug.is_empty() {
            anyhow::bail!("manifest has no repository URL for skill install");
        }
        let repo_dir = CONFIG_DIR.join(".cache").join("repos").join(&slug);
        std::fs::create_dir_all(repo_dir.parent().context("repo cache path has no parent")?)
            .context("failed to create repo cache dir")?;
        git_sync(&manifest.package.repository, &repo_dir)
            .await
            .with_context(|| format!("failed to sync repo {}", &manifest.package.repository))?;

        let skills_dir = CONFIG_DIR.join("skills");
        std::fs::create_dir_all(&skills_dir).context("failed to create skills dir")?;

        for key in &all_skill_keys {
            let skill = manifest.skills.get(*key).ok_or_else(|| {
                anyhow::anyhow!("agent references skill '{key}' not found in [skills]")
            })?;
            on_step(&format!("installing skill {key}…"));

            let src = repo_dir.join(skill.path.as_str());
            let dst = skills_dir.join(key.as_str());
            if dst.exists() {
                std::fs::remove_dir_all(&dst)
                    .with_context(|| format!("failed to remove old skill {key}"))?;
            }
            copy_dir_all(&src, &dst).with_context(|| format!("failed to copy skill {key}"))?;
        }
    }

    // Install agents (copy prompt .md files).
    if !wanted_agents.is_empty() {
        on_step("installing agents…");
        install_agents_filtered(scope, &wanted_agents)?;
    }

    // Register commands (merge metadata into config.toml).
    let wanted_cmds: Vec<_> = manifest
        .commands
        .iter()
        .filter(|(k, _)| filter.wants_command(k.as_str()))
        .collect();
    if !wanted_cmds.is_empty() {
        on_step("registering commands…");
        merge_section_filtered("commands", &wanted_cmds)?;
    }

    Ok(())
}

/// Uninstall a hub package.
///
/// Reads the manifest from the local hub repo (no network sync), removes MCP
/// servers from `config.toml`, deletes skill directories and agent prompt files.
///
/// When `filters` is non-empty, only matching components are removed.
/// Progress messages are reported via `on_step`.
pub async fn uninstall(package: &str, filters: &[String], on_step: impl Fn(&str)) -> Result<()> {
    let filter = HubFilter::parse(filters);

    let (scope, name) = parse_package(package)?;
    let manifest = read_manifest(scope, name)?;

    let mcp_keys: Vec<_> = manifest
        .mcps
        .keys()
        .filter(|k| filter.wants_mcp(k.as_str()))
        .collect();
    if !mcp_keys.is_empty() {
        on_step("removing MCP servers…");
        remove_keys_from_section("mcps", &mcp_keys)?;
    }

    // Collect agent-declared skill keys (mirrors install logic).
    let wanted_agents: Vec<_> = manifest
        .agents
        .iter()
        .filter(|(k, _)| filter.wants_agent(k.as_str()))
        .collect();
    let mut agent_skill_keys: BTreeSet<&String> = BTreeSet::new();
    for (_, agent) in &wanted_agents {
        for sk in &agent.skills {
            agent_skill_keys.insert(sk);
        }
    }

    let skill_keys: BTreeSet<&String> = manifest
        .skills
        .keys()
        .filter(|k| filter.wants_skill(k.as_str()))
        .chain(agent_skill_keys.iter().copied())
        .collect();
    if !skill_keys.is_empty() {
        on_step("removing skills…");
        let skills_dir = CONFIG_DIR.join("skills");
        for key in &skill_keys {
            let dst = skills_dir.join(key.as_str());
            if dst.exists() {
                std::fs::remove_dir_all(&dst)
                    .with_context(|| format!("failed to remove skill {key}"))?;
            }
        }
    }

    if !wanted_agents.is_empty() {
        on_step("removing agents…");
        let agents_dir = CONFIG_DIR.join(wcore::paths::AGENTS_DIR);
        for (name, _) in &wanted_agents {
            let dst = agents_dir.join(format!("{name}.md"));
            if dst.exists() {
                std::fs::remove_file(&dst)
                    .with_context(|| format!("failed to remove agent prompt {}", dst.display()))?;
            }
        }
    }

    let cmd_keys: Vec<_> = manifest
        .commands
        .keys()
        .filter(|k| filter.wants_command(k.as_str()))
        .collect();
    if !cmd_keys.is_empty() {
        on_step("removing commands…");
        remove_keys_from_section("commands", &cmd_keys)?;
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────

/// Ensure `dest` is a shallow clone of `url`, creating or updating as needed.
pub async fn git_sync(url: &str, dest: &Path) -> Result<()> {
    use tokio::process::Command;

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

/// Merge serializable entries into a named section of `config.toml`.
fn merge_section_filtered<T: serde::Serialize>(
    section: &str,
    entries: &[(&String, &T)],
) -> Result<()> {
    use toml_edit::DocumentMut;

    let config_path = CONFIG_DIR.join(wcore::paths::CONFIG_FILE);
    let content = std::fs::read_to_string(&config_path)
        .with_context(|| format!("cannot read {}", config_path.display()))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("invalid TOML in {}", config_path.display()))?;

    let table = doc
        .entry(section)
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .with_context(|| format!("{section} is not a table"))?;

    for (key, cfg) in entries {
        let doc = toml_edit::ser::to_document(cfg)
            .with_context(|| format!("failed to serialize entry for {key}"))?;
        let item = toml_edit::Item::Table(doc.as_table().clone());
        table.insert(key.as_str(), item);
    }

    std::fs::write(&config_path, doc.to_string())
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(())
}

/// Remove keys from a TOML section in `config.toml`.
fn remove_keys_from_section(section: &str, keys: &[&String]) -> Result<()> {
    use toml_edit::DocumentMut;

    let config_path = CONFIG_DIR.join(wcore::paths::CONFIG_FILE);
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
    agents: &[(&String, &manifest::AgentResource)],
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

/// Convert a repo URL to a filesystem-safe slug.
///
/// e.g. `https://github.com/microsoft/playwright-cli` → `github-com-microsoft-playwright-cli`
fn repo_slug(url: &str) -> String {
    url.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
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
