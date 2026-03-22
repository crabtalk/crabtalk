//! Package manifest loading and resolution.
//!
//! Provides [`ManifestConfig`] for parsing `CrabTalk.toml` / package manifests,
//! and [`resolve_manifests`] for merging all installed manifests into a single
//! [`ResolvedManifest`] at startup.

use crate::{
    AgentConfig, McpServerConfig,
    paths::{AGENTS_DIR, LOCAL_DIR, PACKAGES_DIR, SKILLS_DIR},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

// ── Manifest types ──────────────────────────────────────────────────

/// Package manifest format shared by `local/CrabTalk.toml` and
/// `packages/scope/name.toml`. Same shape as hub manifests.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestConfig {
    /// Package metadata (optional for local manifest).
    #[serde(default)]
    pub package: Option<PackageMeta>,
    /// MCP server configurations.
    #[serde(default)]
    pub mcps: BTreeMap<String, McpServerConfig>,
    /// Per-agent configurations (name → config).
    #[serde(default)]
    pub agents: BTreeMap<String, AgentConfig>,
}

/// Package metadata in a manifest.
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

/// Package setup — either a shell command or a prompt for inference.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Setup {
    /// Shell command run from the cached repo directory.
    Command { command: String },
    /// Prompt sent to the daemon for inference. If the value ends with `.md`,
    /// it is read as a file path relative to the repo root.
    Prompt { prompt: String },
}

impl ManifestConfig {
    /// Load a manifest from a TOML file. Returns `None` if file doesn't exist.
    pub fn load(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read manifest at {}", path.display()))?;
        let manifest: Self = toml::from_str(&content)
            .with_context(|| format!("invalid manifest at {}", path.display()))?;
        Ok(Some(manifest))
    }
}

// ── Resolved manifest ───────────────────────────────────────────────

/// Merged view of all manifests (local + packages). Built at daemon startup.
#[derive(Debug, Default)]
pub struct ResolvedManifest {
    /// All MCP server configs (local wins over packages).
    pub mcps: BTreeMap<String, McpServerConfig>,
    /// All agent configs from manifests (local wins over packages).
    pub agents: BTreeMap<String, AgentConfig>,
    /// Skill directories to scan (local first, then packages).
    pub skill_dirs: Vec<PathBuf>,
    /// Agent directories to scan (local first, then packages).
    pub agent_dirs: Vec<PathBuf>,
}

/// Resolve all manifests into a single merged view.
///
/// Loads `local/CrabTalk.toml` first (highest priority), then scans
/// `packages/*/*.toml`. For each package with a `repository` field, resolves
/// skill and agent directories from `.cache/repos/{slug}/`. Local always wins
/// on name conflicts; between packages, first alphabetically wins.
pub fn resolve_manifests(config_dir: &Path) -> ResolvedManifest {
    let mut resolved = ResolvedManifest::default();

    // Local dirs always come first.
    let local_skills = config_dir.join(SKILLS_DIR);
    if local_skills.exists() {
        resolved.skill_dirs.push(local_skills);
    }
    let local_agents = config_dir.join(AGENTS_DIR);
    if local_agents.exists() {
        resolved.agent_dirs.push(local_agents);
    }

    // Load local manifest.
    let local_manifest_path = config_dir.join(LOCAL_DIR).join("CrabTalk.toml");
    if let Ok(Some(manifest)) = ManifestConfig::load(&local_manifest_path) {
        merge_manifest(&mut resolved, &manifest, "local");
    }

    // Scan packages/*/*.toml (sorted for deterministic order).
    let packages_dir = config_dir.join(PACKAGES_DIR);
    if let Ok(scopes) = std::fs::read_dir(&packages_dir) {
        let mut scope_entries: Vec<_> = scopes.flatten().collect();
        scope_entries.sort_by_key(|e| e.file_name());

        for scope_entry in scope_entries {
            let scope_path = scope_entry.path();
            if !scope_path.is_dir() {
                if scope_path.extension().is_some_and(|e| e == "toml") {
                    load_package_manifest(config_dir, &scope_path, &mut resolved);
                }
                continue;
            }
            if let Ok(packages) = std::fs::read_dir(&scope_path) {
                let mut pkg_entries: Vec<_> = packages.flatten().collect();
                pkg_entries.sort_by_key(|e| e.file_name());

                for pkg_entry in pkg_entries {
                    let pkg_path = pkg_entry.path();
                    if pkg_path.extension().is_some_and(|e| e == "toml") {
                        load_package_manifest(config_dir, &pkg_path, &mut resolved);
                    }
                }
            }
        }
    }

    // External tool skill directories (lowest priority).
    if let Some(ref home) = dirs::home_dir() {
        for dir in [".claude/skills", ".codex/skills", ".openclaw/skills"] {
            let path = home.join(dir);
            if path.exists() {
                resolved.skill_dirs.push(path);
            }
        }
    }

    resolved
}

/// Load a single package manifest and merge it into the resolved state.
fn load_package_manifest(config_dir: &Path, path: &Path, resolved: &mut ResolvedManifest) {
    let source = path
        .strip_prefix(config_dir.join(PACKAGES_DIR))
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned();

    let manifest = match ManifestConfig::load(path) {
        Ok(Some(m)) => m,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!("failed to load package manifest {}: {e}", path.display());
            return;
        }
    };

    // Resolve cached repo dirs for skills/agents.
    if let Some(ref pkg) = manifest.package
        && !pkg.repository.is_empty()
    {
        let slug = repo_slug(&pkg.repository);
        let repo_dir = config_dir.join(".cache").join("repos").join(&slug);
        if repo_dir.exists() {
            // Push repo root — skills are discovered recursively by SKILL.md.
            resolved.skill_dirs.push(repo_dir.clone());
            let agents = repo_dir.join("agents");
            if agents.exists() && agents.is_dir() {
                resolved.agent_dirs.push(agents);
            }
        }
    }

    merge_manifest(resolved, &manifest, &source);
}

/// Merge a manifest's MCPs and agents into resolved, skipping duplicates.
fn merge_manifest(resolved: &mut ResolvedManifest, manifest: &ManifestConfig, source: &str) {
    for (name, mcp) in &manifest.mcps {
        if resolved.mcps.contains_key(name) {
            tracing::warn!(
                "MCP '{name}' from {source} conflicts with already-loaded MCP, skipping"
            );
        } else {
            let mut cfg = mcp.clone();
            if cfg.name.is_empty() {
                cfg.name = name.clone();
            }
            resolved.mcps.insert(name.clone(), cfg);
        }
    }

    for (name, agent) in &manifest.agents {
        if resolved.agents.contains_key(name) {
            tracing::warn!(
                "agent '{name}' from {source} conflicts with already-loaded agent, skipping"
            );
        } else {
            resolved.agents.insert(name.clone(), agent.clone());
        }
    }
}

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
        let content = std::fs::read_to_string(entry.path())?;
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
