//! Filesystem-backed [`Storage`] implementation.
//!
//! Single struct holding all paths and state. Replaces the four
//! separate Fs*Repo structs and the FsStorage composite.

use anyhow::Result;
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::ErrorKind,
    path::PathBuf,
    sync::Mutex,
};
use wcore::{
    AgentConfig, AgentId, ArchiveSegment, ConversationMeta, EventLine, ManifestConfig, NodeConfig,
    model::HistoryEntry,
    repos::{MemoryEntry, SessionHandle, SessionSnapshot, SessionSummary, Skill, Storage, slugify},
};

use super::atomic_write;

/// Filesystem persistence backend.
pub struct FsStorage {
    /// Config directory root (for agent prompt storage under `agents/<ulid>/`).
    config_dir: PathBuf,
    /// Root for memory entries and MEMORY.md index.
    memory_root: PathBuf,
    /// Root for session directories.
    sessions_root: PathBuf,
    /// Ordered skill roots to scan (local first, then packages).
    skill_roots: Vec<PathBuf>,
    /// Skill names to exclude.
    disabled_skills: Vec<String>,
    /// Directories containing `<name>.md` legacy agent prompt files.
    agent_dirs: Vec<PathBuf>,
    /// Per-session step counters, recovered from disk on first access.
    session_counters: Mutex<HashMap<String, u64>>,
}

impl FsStorage {
    pub fn new(
        config_dir: PathBuf,
        memory_root: PathBuf,
        sessions_root: PathBuf,
        skill_roots: Vec<PathBuf>,
        disabled_skills: Vec<String>,
        agent_dirs: Vec<PathBuf>,
    ) -> Self {
        Self {
            config_dir,
            memory_root,
            sessions_root,
            skill_roots,
            disabled_skills,
            agent_dirs,
            session_counters: Mutex::new(HashMap::new()),
        }
    }

    // ── Memory helpers ─────────────────────────────────────────────

    fn memory_entries_dir(&self) -> PathBuf {
        self.memory_root.join("entries")
    }

    fn memory_entry_path(&self, name: &str) -> PathBuf {
        self.memory_entries_dir()
            .join(format!("{}.md", slugify(name)))
    }

    fn memory_index_path(&self) -> PathBuf {
        self.memory_root.join("MEMORY.md")
    }

    // ── Session helpers ────────────────────────────────────────────

    fn session_dir(&self, slug: &str) -> PathBuf {
        self.sessions_root.join(slug)
    }

    fn session_meta_path(&self, slug: &str) -> PathBuf {
        self.session_dir(slug).join("meta")
    }

    fn session_step_path(&self, slug: &str, step: u64) -> PathBuf {
        self.session_dir(slug).join(format!("step-{step:06}"))
    }

    fn next_step(&self, slug: &str) -> u64 {
        let mut counters = self.session_counters.lock().unwrap();
        let counter = counters
            .entry(slug.to_owned())
            .or_insert_with(|| recover_step_counter(&self.session_dir(slug)));
        let n = *counter;
        *counter += 1;
        n
    }

    fn write_step(&self, slug: &str, line: StepLine) -> Result<()> {
        let step = self.next_step(slug);
        let path = self.session_step_path(slug, step);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(&line)?;
        atomic_write(&path, &bytes)
    }

    // ── Agent helpers ──────────────────────────────────────────────

    fn agent_prompt_path(&self, id: &AgentId) -> PathBuf {
        self.config_dir
            .join("agents")
            .join(id.to_string())
            .join("prompt.md")
    }
}

// ── Step serialization ─────────────────────────────────────────────

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum StepLine {
    Compact {
        compact: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        title: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        archived_at: String,
    },
    Event(EventLine),
    Entry(HistoryEntry),
}

// ── Storage implementation ─────────────────────────────────────────

impl Storage for FsStorage {
    // ── Memory ─────────────────────────────────────────────────────

    fn list_memories(&self) -> Result<Vec<MemoryEntry>> {
        let dir = self.memory_entries_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut entries = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md") {
                let content = fs::read_to_string(&path)?;
                match MemoryEntry::parse(&content) {
                    Ok(parsed) => entries.push(parsed),
                    Err(e) => tracing::warn!("failed to parse {}: {e}", path.display()),
                }
            }
        }
        Ok(entries)
    }

    fn load_memory(&self, name: &str) -> Result<Option<MemoryEntry>> {
        let path = self.memory_entry_path(name);
        match fs::read_to_string(&path) {
            Ok(content) => Ok(Some(MemoryEntry::parse(&content)?)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn save_memory(&self, entry: &MemoryEntry) -> Result<()> {
        let path = self.memory_entry_path(&entry.name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_write(&path, entry.serialize().as_bytes())
    }

    fn delete_memory(&self, name: &str) -> Result<bool> {
        let path = self.memory_entry_path(name);
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    fn load_memory_index(&self) -> Result<Option<String>> {
        let path = self.memory_index_path();
        match fs::read_to_string(&path) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn save_memory_index(&self, content: &str) -> Result<()> {
        let path = self.memory_index_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_write(&path, content.as_bytes())
    }

    // ── Skills ─────────────────────────────────────────────────────

    fn list_skills(&self) -> Result<Vec<Skill>> {
        let mut skills = Vec::new();
        let mut seen = HashSet::new();
        for root in &self.skill_roots {
            if !root.exists() {
                continue;
            }
            let entries = match fs::read_dir(root) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) if !n.starts_with('.') => n.to_owned(),
                    _ => continue,
                };
                if seen.contains(&name) || self.disabled_skills.contains(&name) {
                    continue;
                }
                let skill_path = path.join("SKILL.md");
                if !skill_path.exists() {
                    continue;
                }
                let content = match fs::read_to_string(&skill_path) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!("failed to read {}: {e}", skill_path.display());
                        continue;
                    }
                };
                match tools::skill::loader::parse_skill_md(&content) {
                    Ok(skill) => {
                        seen.insert(name);
                        skills.push(skill);
                    }
                    Err(e) => tracing::warn!("failed to parse {}: {e}", skill_path.display()),
                }
            }
        }
        Ok(skills)
    }

    fn load_skill(&self, name: &str) -> Result<Option<Skill>> {
        for root in &self.skill_roots {
            let skill_path = root.join(name).join("SKILL.md");
            if !skill_path.exists() {
                continue;
            }
            let content = fs::read_to_string(&skill_path)?;
            let skill = tools::skill::loader::parse_skill_md(&content)?;
            return Ok(Some(skill));
        }
        Ok(None)
    }

    // ── Sessions ───────────────────────────────────────────────────

    fn create_session(&self, agent: &str, created_by: &str) -> Result<SessionHandle> {
        let agent_slug = wcore::sender_slug(agent);
        let sender = wcore::sender_slug(created_by);
        let prefix = format!("{agent_slug}_{sender}_");
        let seq = next_session_seq(&self.sessions_root, &prefix);
        let slug = format!("{agent_slug}_{sender}_{seq}");

        let dir = self.session_dir(&slug);
        fs::create_dir_all(&dir)?;

        let meta = ConversationMeta {
            agent: agent.to_owned(),
            created_by: created_by.to_owned(),
            created_at: chrono::Utc::now().to_rfc3339(),
            title: String::new(),
            uptime_secs: 0,
        };
        let meta_bytes = serde_json::to_vec(&meta)?;
        atomic_write(&self.session_meta_path(&slug), &meta_bytes)?;
        Ok(SessionHandle::new(slug))
    }

    fn find_latest_session(&self, agent: &str, created_by: &str) -> Result<Option<SessionHandle>> {
        let agent_slug = wcore::sender_slug(agent);
        let sender = wcore::sender_slug(created_by);
        let prefix = format!("{agent_slug}_{sender}_");

        if !self.sessions_root.exists() {
            return Ok(None);
        }

        let mut best: Option<(u32, String)> = None;
        for entry in fs::read_dir(&self.sessions_root)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with(&prefix) || !entry.file_type()?.is_dir() {
                continue;
            }
            let seq_str = &name[prefix.len()..];
            if let Ok(seq) = seq_str.parse::<u32>()
                && best.as_ref().is_none_or(|(b, _)| seq > *b)
            {
                best = Some((seq, name.to_string()));
            }
        }
        Ok(best.map(|(_, slug)| SessionHandle::new(slug)))
    }

    fn load_session(&self, handle: &SessionHandle) -> Result<Option<SessionSnapshot>> {
        let slug = handle.as_str();
        let meta_path = self.session_meta_path(slug);
        let meta_bytes = match fs::read(&meta_path) {
            Ok(b) => b,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let meta: ConversationMeta = serde_json::from_slice(&meta_bytes)?;

        let dir = self.session_dir(slug);
        let mut step_files: Vec<_> = fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with("step-"))
            })
            .collect();
        step_files.sort_by_key(|e| e.file_name());

        let mut lines = Vec::with_capacity(step_files.len());
        let mut last_compact_idx: Option<usize> = None;
        for entry in &step_files {
            let bytes = fs::read(entry.path())?;
            match serde_json::from_slice::<StepLine>(&bytes) {
                Ok(line) => {
                    if matches!(line, StepLine::Compact { .. }) {
                        last_compact_idx = Some(lines.len());
                    }
                    lines.push(line);
                }
                Err(e) => {
                    tracing::warn!("skipping unparsable step {}: {e}", entry.path().display());
                }
            }
        }

        let start = last_compact_idx.unwrap_or(0);
        let mut history = Vec::new();
        for (i, line) in lines[start..].iter().enumerate() {
            match line {
                StepLine::Compact { compact, .. } if i == 0 && last_compact_idx.is_some() => {
                    history.push(HistoryEntry::user(compact));
                }
                StepLine::Entry(entry) => history.push(entry.clone()),
                StepLine::Event(_) | StepLine::Compact { .. } => {}
            }
        }

        Ok(Some(SessionSnapshot { meta, history }))
    }

    fn load_session_archives(&self, handle: &SessionHandle) -> Result<Vec<ArchiveSegment>> {
        let dir = self.session_dir(handle.as_str());
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut step_files: Vec<_> = fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with("step-"))
            })
            .collect();
        step_files.sort_by_key(|e| e.file_name());

        let mut archives = Vec::new();
        for entry in step_files {
            let bytes = fs::read(entry.path())?;
            if let Ok(StepLine::Compact {
                compact,
                title,
                archived_at,
            }) = serde_json::from_slice(&bytes)
            {
                archives.push(ArchiveSegment {
                    title,
                    summary: compact,
                    archived_at,
                });
            }
        }
        Ok(archives)
    }

    fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        if !self.sessions_root.exists() {
            return Ok(Vec::new());
        }
        let mut summaries = Vec::new();
        for entry in fs::read_dir(&self.sessions_root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let slug = entry.file_name().to_string_lossy().to_string();
            let meta_path = self.session_meta_path(&slug);
            if let Ok(bytes) = fs::read(&meta_path)
                && let Ok(meta) = serde_json::from_slice::<ConversationMeta>(&bytes)
            {
                summaries.push(SessionSummary {
                    handle: SessionHandle::new(slug),
                    meta,
                });
            }
        }
        Ok(summaries)
    }

    fn append_session_messages(
        &self,
        handle: &SessionHandle,
        entries: &[HistoryEntry],
    ) -> Result<()> {
        for entry in entries {
            self.write_step(handle.as_str(), StepLine::Entry(entry.clone()))?;
        }
        Ok(())
    }

    fn append_session_events(&self, handle: &SessionHandle, events: &[EventLine]) -> Result<()> {
        for event in events {
            self.write_step(handle.as_str(), StepLine::Event(event.clone()))?;
        }
        Ok(())
    }

    fn append_session_compact(&self, handle: &SessionHandle, summary: &str) -> Result<()> {
        let line = StepLine::Compact {
            compact: summary.to_owned(),
            title: compact_title(summary),
            archived_at: chrono::Utc::now().to_rfc3339(),
        };
        self.write_step(handle.as_str(), line)
    }

    fn update_session_meta(&self, handle: &SessionHandle, meta: &ConversationMeta) -> Result<()> {
        let path = self.session_meta_path(handle.as_str());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(meta)?;
        atomic_write(&path, &bytes)
    }

    fn delete_session(&self, handle: &SessionHandle) -> Result<bool> {
        let dir = self.session_dir(handle.as_str());
        match fs::remove_dir_all(&dir) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    // ── Agents ─────────────────────────────────────────────────────

    fn list_agents(&self) -> Result<Vec<AgentConfig>> {
        Ok(Vec::new())
    }

    fn load_agent(&self, id: &AgentId) -> Result<Option<AgentConfig>> {
        if id.is_nil() {
            return Ok(None);
        }
        let path = self.agent_prompt_path(id);
        match fs::read_to_string(&path) {
            Ok(prompt) => Ok(Some(AgentConfig {
                id: *id,
                system_prompt: prompt,
                ..Default::default()
            })),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn load_agent_by_name(&self, name: &str) -> Result<Option<AgentConfig>> {
        for dir in &self.agent_dirs {
            let path = dir.join(format!("{name}.md"));
            if let Ok(prompt) = fs::read_to_string(&path) {
                let mut config = AgentConfig::new(name);
                config.system_prompt = prompt;
                return Ok(Some(config));
            }
        }
        Ok(None)
    }

    fn upsert_agent(&self, config: &AgentConfig, prompt: &str) -> Result<()> {
        if config.id.is_nil() {
            anyhow::bail!("cannot upsert agent with nil ID");
        }
        let path = self.agent_prompt_path(&config.id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_write(&path, prompt.as_bytes())
    }

    fn delete_agent(&self, id: &AgentId) -> Result<bool> {
        let dir = self.config_dir.join("agents").join(id.to_string());
        match fs::remove_dir_all(&dir) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    fn rename_agent(&self, _id: &AgentId, _new_name: &str) -> Result<bool> {
        // Rename is a manifest-level operation (change the TOML key).
        // The ULID stays stable, so the prompt file doesn't move.
        Ok(true)
    }

    // ── Manifest ───────────────────────────────────────────────────

    fn load_local_manifest(&self) -> Result<ManifestConfig> {
        let path = self
            .config_dir
            .join(wcore::paths::LOCAL_DIR)
            .join("CrabTalk.toml");
        match ManifestConfig::load(&path)? {
            Some(m) => Ok(m),
            None => Ok(ManifestConfig::default()),
        }
    }

    fn save_local_manifest(&self, manifest: &ManifestConfig) -> Result<()> {
        let dir = self.config_dir.join(wcore::paths::LOCAL_DIR);
        fs::create_dir_all(&dir)?;
        let content = toml::to_string_pretty(manifest)?;
        atomic_write(&dir.join("CrabTalk.toml"), content.as_bytes())
    }

    fn load_config(&self) -> Result<NodeConfig> {
        let path = self.config_dir.join(wcore::paths::CONFIG_FILE);
        if !path.exists() {
            return Ok(NodeConfig::default());
        }
        NodeConfig::load(&path)
    }

    fn save_config(&self, config: &NodeConfig) -> Result<()> {
        let path = self.config_dir.join(wcore::paths::CONFIG_FILE);
        let content = toml::to_string_pretty(config)?;
        atomic_write(&path, content.as_bytes())
    }

    fn scaffold(&self) -> Result<()> {
        fs::create_dir_all(&self.config_dir)?;
        fs::create_dir_all(self.config_dir.join(wcore::paths::LOCAL_DIR))?;
        fs::create_dir_all(self.config_dir.join(wcore::paths::SKILLS_DIR))?;
        fs::create_dir_all(self.config_dir.join(wcore::paths::AGENTS_DIR))?;
        fs::create_dir_all(&self.memory_root)?;
        fs::create_dir_all(&self.sessions_root)?;
        Ok(())
    }
}

// ── Free functions ─────────────────────────────────────────────────

fn recover_step_counter(dir: &PathBuf) -> u64 {
    let mut max = 0u64;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(suffix) = name.strip_prefix("step-")
                && let Ok(n) = suffix.parse::<u64>()
            {
                max = max.max(n);
            }
        }
    }
    max + 1
}

fn next_session_seq(root: &PathBuf, prefix: &str) -> u32 {
    let mut max = 0u32;
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(seq_str) = name.strip_prefix(prefix)
                && let Ok(seq) = seq_str.parse::<u32>()
            {
                max = max.max(seq);
            }
        }
    }
    max + 1
}

fn compact_title(summary: &str) -> String {
    let end = summary
        .find(['.', '!', '?'])
        .map(|i| i + 1)
        .unwrap_or(summary.len())
        .min(60);
    let title = summary[..summary.floor_char_boundary(end)].trim();
    title.to_string()
}
