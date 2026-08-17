//! Package directory discovery.
//!
//! Scans `packages/*.toml` to find each package's cached repo (skills,
//! agents, MCPs). User-added agents and MCPs live in Storage; this
//! module only resolves filesystem locations for packaged content.

use crate::{
    AgentConfig, McpServerConfig,
    paths::{AGENTS_DIR, PACKAGES_DIR, SKILLS_DIR},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

// ── Package manifest types ──────────────────────────────────────────

/// Subset of a package TOML this module reads at boot:
///   - `[package]` for repo slug → cached repo dir
///   - `[mcps.<name>]` and `[agents.<name>]` shipped by the package
///
/// Mutable per-user state lives in [`crate::storage::Storage`]; package
/// manifests are read-only on-disk packaging that's re-read every boot.
#[derive(Debug, Clone, Default, Deserialize)]
struct PackageManifest {
    #[serde(default)]
    package: Option<PackageMeta>,
    #[serde(default)]
    mcps: BTreeMap<String, McpServerConfig>,
    #[serde(default)]
    agents: BTreeMap<String, AgentConfig>,
}

/// Package metadata in a package manifest.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackageMeta {
    /// Package name.
    #[serde(default)]
    pub name: String,
    /// Source repository URL.
    #[serde(default)]
    pub repository: String,
    /// Branch to clone (defaults to the repo's default branch).
    #[serde(default)]
    pub branch: Option<String>,
    /// Setup configuration (run after install).
    #[serde(default)]
    pub setup: Option<Setup>,
}

/// Package setup — a bash script run after install.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Setup {
    /// Bash script or command to run from the cached repo directory.
    pub script: String,
}

// ── Resolved directories ────────────────────────────────────────────

/// Directories discovered at startup: local + package repos + external.
/// Used to seed [`crate::storage::Storage`] with skill/agent roots and
/// to surface MCPs/agents that ship inside a package TOML.
#[derive(Debug, Default)]
pub struct ResolvedDirs {
    /// Skill directories to scan (local first, then packages, then external).
    pub skill_dirs: Vec<PathBuf>,
    /// Agent directories to scan (local first, then packages).
    pub agent_dirs: Vec<PathBuf>,
    /// Package name → skill directory for resolving qualified skill
    /// references (e.g. `"playwright"` → repo skill dir).
    pub package_skill_dirs: BTreeMap<String, PathBuf>,
    /// MCP servers declared in `packages/*.toml` `[mcps.<name>]` blocks.
    /// Storage MCPs (user-defined) win on name conflict.
    pub package_mcps: BTreeMap<String, McpServerConfig>,
    /// Agents declared in `packages/*.toml` `[agents.<name>]` blocks.
    /// Storage agents (user-defined) win on name conflict.
    pub package_agents: BTreeMap<String, AgentConfig>,
}

/// Discover skill, agent, and package directories under `config_dir`.
///
/// Walks `local/skills`, `local/agents`, every `packages/*.toml` (whose
/// `[package].repository` resolves to a cached repo under `.cache/repos/`),
/// and well-known external skill roots (`~/.claude/skills` etc.).
pub fn resolve_dirs(config_dir: &Path) -> ResolvedDirs {
    let mut resolved = ResolvedDirs::default();

    let local_skills = config_dir.join(SKILLS_DIR);
    if local_skills.exists() {
        resolved.skill_dirs.push(local_skills);
    }
    let local_agents = config_dir.join(AGENTS_DIR);
    if local_agents.exists() {
        resolved.agent_dirs.push(local_agents);
    }

    let packages_dir = config_dir.join(PACKAGES_DIR);
    if let Ok(entries) = std::fs::read_dir(&packages_dir) {
        let mut package_entries: Vec<_> = entries.flatten().collect();
        package_entries.sort_by_key(|e| e.file_name());

        for entry in package_entries {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "toml") {
                load_package_dirs(config_dir, &path, &mut resolved);
            }
        }
    }

    if let Some(home) = dirs::home_dir() {
        for dir in [".claude/skills", ".codex/skills", ".openclaw/skills"] {
            let path = home.join(dir);
            if path.exists() {
                resolved.skill_dirs.push(path);
            }
        }
    }

    resolved
}

fn load_package_dirs(config_dir: &Path, path: &Path, resolved: &mut ResolvedDirs) {
    let source = path
        .strip_prefix(config_dir.join(PACKAGES_DIR))
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned();
    let package_id = source.strip_suffix(".toml").unwrap_or(&source).to_owned();

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("failed to read package manifest {}: {e}", path.display());
            return;
        }
    };
    let manifest: PackageManifest = match toml::from_str(&content) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("failed to parse package manifest {}: {e}", path.display());
            return;
        }
    };

    for (name, mut mcp) in manifest.mcps {
        if mcp.name.is_empty() {
            mcp.name = name.clone();
        }
        resolved.package_mcps.entry(name).or_insert(mcp);
    }
    for (name, mut agent) in manifest.agents {
        agent.name = name.clone();
        resolved.package_agents.entry(name).or_insert(agent);
    }

    let Some(pkg) = manifest.package else {
        return;
    };
    if pkg.repository.is_empty() {
        return;
    }
    let slug = repo_slug(&pkg.repository);
    let repo_dir = config_dir.join(".cache").join("repos").join(&slug);
    if !repo_dir.exists() {
        return;
    }
    resolved.skill_dirs.push(repo_dir.clone());
    resolved
        .package_skill_dirs
        .insert(package_id, repo_dir.clone());
    let agents = repo_dir.join("agents");
    if agents.exists() && agents.is_dir() {
        resolved.agent_dirs.push(agents);
    }
}

/// Derive the external source name from a skill directory path.
///
/// For `~/.claude/skills` the source name is `"claude"` (the parent
/// directory name with the leading `.` stripped). Returns `None` for
/// paths that don't match the `~/.<name>/skills` pattern.
pub fn external_source_name(path: &Path) -> Option<&str> {
    path.components()
        .rev()
        .nth(1)
        .and_then(|c| c.as_os_str().to_str())
        .and_then(|s| s.strip_prefix('.'))
}

// ── Skill conflict detection ────────────────────────────────────────

/// Convert a repo URL to a filesystem-safe slug.
///
/// e.g. `https://github.com/microsoft/playwright-cli` → `github-com-microsoft-playwright-cli`
pub fn repo_slug(url: &str) -> String {
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

// ── Agent loading ───────────────────────────────────────────────────

/// Load all agent markdown files from a directory as plain text.
///
/// Returns `(filename_stem, content)` pairs. Non-`.md` files are silently
/// skipped. Entries are sorted by filename for deterministic ordering.
/// Returns an empty vec if the directory does not exist.
pub fn load_agents_dir(path: &Path) -> Result<Vec<(String, String)>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<_> = std::fs::read_dir(path)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut agents = Vec::with_capacity(entries.len());
    for entry in entries {
        let stem = entry
            .path()
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let content = std::fs::read_to_string(entry.path())
            .with_context(|| format!("read {}", entry.path().display()))?;
        agents.push((stem, content));
    }

    Ok(agents)
}

/// Load agents from multiple directories, concatenating results.
/// First directory's agents win on name conflicts (local-first ordering).
pub fn load_agents_dirs(dirs: &[PathBuf]) -> Result<Vec<(String, String)>> {
    let mut seen = std::collections::BTreeSet::new();
    let mut all = Vec::new();
    for dir in dirs {
        for (stem, content) in load_agents_dir(dir)? {
            if seen.insert(stem.clone()) {
                all.push((stem, content));
            }
        }
    }
    Ok(all)
}
