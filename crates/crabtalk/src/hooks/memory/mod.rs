//! Memory hook — thin facade over `crabtalk-memory`, plus the system
//! prompt assembly and legacy-tree import for first-open migration.

use anyhow::Result;
use memory::{EntryKind, Memory as Store, Op};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};
use wcore::MemoryConfig;
use wcore::model::{HistoryEntry, Role};

/// Shared handle to the underlying memory store. Cloneable because the
/// runtime needs a reference of its own for writing archives during
/// compaction and reading them back on session resume.
pub type SharedStore = Arc<RwLock<Store>>;

pub mod handlers;

const MEMORY_PROMPT: &str = include_str!("../../../prompts/memory.md");
pub const DEFAULT_SOUL: &str = include_str!("../../../prompts/crab.md");

/// Reserved entry name for the always-injected curated overview — what
/// used to be `MEMORY.md`. Named `global` because per-agent prompts
/// (v2) will live as sibling entries keyed by agent id.
pub const GLOBAL_PROMPT_NAME: &str = "global";

/// Reserved names users can't create/delete through `remember`/`forget`
/// — their content is load-bearing for the agent's system prompt.
fn is_reserved(name: &str) -> bool {
    name == GLOBAL_PROMPT_NAME
}

pub struct Memory {
    inner: SharedStore,
    recall_limit: usize,
}

impl Memory {
    /// Open the memory db at `db_path`. If the db is fresh and
    /// `legacy_dir` points to an old `memory/` directory (frontmatter
    /// entries + `MEMORY.md`), import it in one shot.
    pub fn open(
        config: MemoryConfig,
        db_path: PathBuf,
        legacy_dir: Option<PathBuf>,
    ) -> Result<Self> {
        let fresh = !db_path.exists();
        let mut store = Store::open(&db_path)?;
        if fresh {
            if let Some(dir) = legacy_dir {
                import_legacy(&mut store, &dir)?;
            }
            // Create the db file unconditionally so a crash or a
            // legacy import with zero successful ops still counts
            // as "migrated" — next open won't re-enter this branch.
            store.checkpoint()?;
        }
        Ok(Self {
            inner: Arc::new(RwLock::new(store)),
            recall_limit: config.recall_limit,
        })
    }

    /// Clone the underlying store handle. Used to hand the same memory
    /// to the runtime for archive writes and resume-time reads.
    pub fn shared(&self) -> SharedStore {
        self.inner.clone()
    }

    pub fn recall(&self, query: &str, limit: usize) -> String {
        let store = self.inner.read().unwrap();
        let hits = store.search(query, limit);
        if hits.is_empty() {
            return "no memories found".to_owned();
        }
        hits.iter()
            .map(|h| format!("## {}\n{}", h.entry.name, h.entry.content))
            .collect::<Vec<_>>()
            .join("\n---\n")
    }

    pub fn remember(&self, name: String, content: String, aliases: Vec<String>) -> String {
        if is_reserved(&name) {
            return format!("'{name}' is reserved — use the memory tool to edit it");
        }
        let mut store = self.inner.write().unwrap();
        let exists = store.get(&name).is_some();
        let op = if exists {
            Op::Update {
                name: name.clone(),
                content,
                aliases,
            }
        } else {
            Op::Add {
                name: name.clone(),
                content,
                aliases,
                kind: EntryKind::Note,
            }
        };
        match store.apply(op) {
            Ok(_) => format!("remembered: {name}"),
            Err(e) => format!("failed to save entry: {e}"),
        }
    }

    pub fn forget(&self, name: &str) -> String {
        if is_reserved(name) {
            return format!("'{name}' is reserved and cannot be forgotten");
        }
        let mut store = self.inner.write().unwrap();
        match store.apply(Op::Remove {
            name: name.to_owned(),
        }) {
            Ok(_) => format!("forgot: {name}"),
            Err(_) => format!("no entry named: {name}"),
        }
    }

    /// Upsert the reserved `global` Prompt entry (what `MEMORY.md` used
    /// to be).
    pub fn write_prompt(&self, content: &str) -> String {
        let mut store = self.inner.write().unwrap();
        let exists = store.get(GLOBAL_PROMPT_NAME).is_some();
        let op = if exists {
            Op::Update {
                name: GLOBAL_PROMPT_NAME.to_owned(),
                content: content.to_owned(),
                aliases: vec![],
            }
        } else {
            Op::Add {
                name: GLOBAL_PROMPT_NAME.to_owned(),
                content: content.to_owned(),
                aliases: vec![],
                kind: EntryKind::Prompt,
            }
        };
        match store.apply(op) {
            Ok(_) => "MEMORY.md updated".to_owned(),
            Err(e) => format!("failed to write MEMORY.md: {e}"),
        }
    }

    /// System-prompt block: the `global` Prompt content wrapped in
    /// `<memory>` tags, plus the memory tool instructions.
    pub fn build_prompt(&self) -> String {
        let store = self.inner.read().unwrap();
        match store.get(GLOBAL_PROMPT_NAME) {
            Some(e) if !e.content.trim().is_empty() => {
                format!("\n\n<memory>\n{}\n</memory>\n\n{MEMORY_PROMPT}", e.content)
            }
            _ => format!("\n\n{MEMORY_PROMPT}"),
        }
    }

    /// Auto-recall: BM25-search the last user message, inject any hits
    /// as a synthetic user turn.
    pub fn before_run(&self, history: &[HistoryEntry]) -> Vec<HistoryEntry> {
        let last_user = history
            .iter()
            .rev()
            .find(|e| *e.role() == Role::User && !e.text().is_empty());

        let Some(entry) = last_user else {
            return Vec::new();
        };

        let query: String = entry
            .text()
            .split_whitespace()
            .take(8)
            .collect::<Vec<_>>()
            .join(" ");

        if query.is_empty() {
            return Vec::new();
        }

        let result = self.recall(&query, self.recall_limit);
        if result == "no memories found" {
            return Vec::new();
        }
        vec![HistoryEntry::user(format!("<recall>\n{result}\n</recall>")).auto_injected()]
    }
}

/// Import entries from a legacy `memory/` directory:
/// - `memory/entries/*.md` (YAML frontmatter + body) → Note entries
/// - `memory/MEMORY.md`                              → `global` Prompt entry
///
/// One-shot, best-effort: malformed files are logged and skipped so one
/// broken entry can't block the upgrade.
fn import_legacy(store: &mut Store, dir: &Path) -> Result<()> {
    let entries_dir = dir.join("entries");
    if entries_dir.is_dir() {
        for ent in std::fs::read_dir(&entries_dir)? {
            let path = ent?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            match parse_legacy_entry(&raw) {
                Some((name, _)) if is_reserved(&name) => {
                    tracing::warn!(
                        ?path,
                        "legacy import: skipping entry with reserved name '{name}'"
                    );
                }
                Some((name, content)) => {
                    if let Err(e) = store.apply(Op::Add {
                        name,
                        content,
                        aliases: vec![],
                        kind: EntryKind::Note,
                    }) {
                        tracing::warn!(?path, "legacy import: {e}");
                    }
                }
                None => tracing::warn!(?path, "legacy import: unparsable entry"),
            }
        }
    }

    let index_path = dir.join("MEMORY.md");
    if let Ok(content) = std::fs::read_to_string(&index_path)
        && !content.trim().is_empty()
        && let Err(e) = store.apply(Op::Add {
            name: GLOBAL_PROMPT_NAME.to_owned(),
            content,
            aliases: vec![],
            kind: EntryKind::Prompt,
        })
    {
        tracing::warn!("legacy MEMORY.md import: {e}");
    }
    Ok(())
}

/// Parse the legacy frontmatter-based entry format into `(name, content)`.
/// Description, if present, is folded into the first line of content.
fn parse_legacy_entry(raw: &str) -> Option<(String, String)> {
    let raw = raw.replace("\r\n", "\n");
    let raw = raw.trim();
    let after_open = raw.strip_prefix("---")?;
    let close_pos = after_open.find("\n---")?;
    let frontmatter = &after_open[..close_pos];
    let body = after_open[close_pos + 4..].trim();

    let mut name = None;
    let mut description: Option<String> = None;
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            name = Some(val.trim().to_owned());
        } else if let Some(val) = line.strip_prefix("description:") {
            description = Some(val.trim().to_owned());
        }
    }

    let name = name?;
    let content = match description.filter(|d| !d.is_empty()) {
        Some(desc) if !body.is_empty() => format!("{desc}\n\n{body}"),
        Some(desc) => desc,
        None => body.to_owned(),
    };
    Some((name, content))
}
