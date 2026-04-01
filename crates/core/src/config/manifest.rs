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
    /// Disabled items — filtered out during runtime build.
    #[serde(default)]
    pub disabled: DisabledItems,
}

/// Items disabled by the user.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DisabledItems {
    #[serde(default)]
    pub providers: Vec<String>,
    #[serde(default)]
    pub mcps: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
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

/// Package setup — a bash script run after install.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Setup {
    /// Bash script or command to run from the cached repo directory.
    pub script: String,
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
    /// Package identifier → skill directory for resolving qualified skill
    /// references (e.g. `"crabtalk/gstack"` → repo skill dir).
    pub package_skill_dirs: BTreeMap<String, PathBuf>,
    /// Items disabled by the user (from local manifest).
    pub disabled: DisabledItems,
}

/// Resolve all manifests into a single merged view.
///
/// Loads `local/CrabTalk.toml` first (highest priority), then scans
/// `packages/*/*.toml`. For each package with a `repository` field, resolves
/// skill and agent directories from `.cache/repos/{slug}/`. Local always wins
/// on name conflicts; between packages, first alphabetically wins.
///
/// Returns the resolved manifest and a list of conflict warnings (if any).
pub fn resolve_manifests(config_dir: &Path) -> (ResolvedManifest, Vec<String>) {
    let mut resolved = ResolvedManifest::default();
    let mut warnings = Vec::new();

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
        // NOTE: disabled items are read from config.toml, not from manifests.
        merge_manifest(&mut resolved, &manifest, "local", &mut warnings);
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
                    load_package_manifest(config_dir, &scope_path, &mut resolved, &mut warnings);
                }
                continue;
            }
            if let Ok(packages) = std::fs::read_dir(&scope_path) {
                let mut pkg_entries: Vec<_> = packages.flatten().collect();
                pkg_entries.sort_by_key(|e| e.file_name());

                for pkg_entry in pkg_entries {
                    let pkg_path = pkg_entry.path();
                    if pkg_path.extension().is_some_and(|e| e == "toml") {
                        load_package_manifest(config_dir, &pkg_path, &mut resolved, &mut warnings);
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

    (resolved, warnings)
}

/// Load a single package manifest and merge it into the resolved state.
fn load_package_manifest(
    config_dir: &Path,
    path: &Path,
    resolved: &mut ResolvedManifest,
    warnings: &mut Vec<String>,
) {
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

    // Derive package identifier by stripping the .toml extension.
    let package_id = source.strip_suffix(".toml").unwrap_or(&source).to_owned();

    // Resolve cached repo dirs for skills/agents.
    if let Some(ref pkg) = manifest.package
        && !pkg.repository.is_empty()
    {
        let slug = repo_slug(&pkg.repository);
        let repo_dir = config_dir.join(".cache").join("repos").join(&slug);
        if repo_dir.exists() {
            // Push repo root — skills are discovered recursively by SKILL.md.
            resolved.skill_dirs.push(repo_dir.clone());
            resolved
                .package_skill_dirs
                .insert(package_id, repo_dir.clone());
            let agents = repo_dir.join("agents");
            if agents.exists() && agents.is_dir() {
                resolved.agent_dirs.push(agents);
            }
        }
    }

    merge_manifest(resolved, &manifest, &source, warnings);
}

/// Merge a manifest's MCPs and agents into resolved, skipping duplicates.
fn merge_manifest(
    resolved: &mut ResolvedManifest,
    manifest: &ManifestConfig,
    source: &str,
    warnings: &mut Vec<String>,
) {
    for (name, mcp) in &manifest.mcps {
        if resolved.mcps.contains_key(name) {
            let msg =
                format!("MCP '{name}' from {source} conflicts with already-loaded MCP, skipping");
            tracing::warn!("{msg}");
            warnings.push(msg);
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
            let msg = format!(
                "agent '{name}' from {source} conflicts with already-loaded agent, skipping"
            );
            tracing::warn!("{msg}");
            warnings.push(msg);
        } else {
            resolved.agents.insert(name.clone(), agent.clone());
        }
    }
}

// ── Skill conflict detection ────────────────────────────────────────

/// Check for skill name conflicts across multiple skill directories.
///
/// Scans each directory for `SKILL.md` files, extracts the `name` field
/// from YAML frontmatter, and reports duplicates. First directory wins
/// (same priority semantics as daemon skill loading).
pub fn check_skill_conflicts(skill_dirs: &[PathBuf]) -> Vec<String> {
    let mut seen = std::collections::BTreeMap::<String, &Path>::new();
    let mut warnings = Vec::new();

    for dir in skill_dirs {
        if !dir.exists() {
            continue;
        }
        for name in scan_skill_names(dir) {
            if let Some(first_dir) = seen.get(&name) {
                warnings.push(format!(
                    "skill '{name}' from {} conflicts with skill from {}, skipping",
                    dir.display(),
                    first_dir.display(),
                ));
            } else {
                seen.insert(name, dir);
            }
        }
    }

    warnings
}

/// Scan a directory recursively for `SKILL.md` files and extract skill names.
pub fn scan_skill_names(dir: &Path) -> Vec<String> {
    let mut results = Vec::new();
    scan_skill_names_inner(dir, &mut results);
    results
}

fn scan_skill_names_inner(dir: &Path, results: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with('.'))
        {
            continue;
        }

        let skill_file = path.join("SKILL.md");
        if skill_file.exists() {
            if let Some(name) = extract_skill_name(&skill_file) {
                results.push(name);
            }
        } else {
            scan_skill_names_inner(&path, results);
        }
    }
}

/// Extract the `name` field from a SKILL.md YAML frontmatter.
fn extract_skill_name(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let (frontmatter, _) = crate::utils::split_yaml_frontmatter(&content).ok()?;
    // Simple line scan — avoids pulling in a YAML parser.
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("name:") {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
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
