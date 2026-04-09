//! Conversation — data container with per-step Storage persistence.
//!
//! Each conversation is scoped to a slug (`{agent_slug}_{sender_slug}_{seq}`)
//! and lays its state out under the runtime [`Storage`] as:
//!
//! - `sessions/<slug>/meta` — single [`ConversationMeta`] JSON blob.
//! - `sessions/<slug>/step-<nnnnnn>` — one key per persisted step
//!   (message, trace event, or compact marker). `nnnnnn` is a
//!   zero-padded monotonic counter so `list(prefix)` returns steps in
//!   insertion order.
//!
//! Compact markers double as archive boundaries:
//! [`Conversation::load_context`] replays from the last compact
//! forward — messages persisted before a compact belong to an
//! archived segment and are not rehydrated into the working context.

use crate::{AgentEvent, AgentStep, Storage, model::HistoryEntry};
use crabllm_core::Usage;
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Prefix for all session-related keys.
const SESSIONS_PREFIX: &str = "sessions/";
/// Suffix for per-step keys under a session slug.
const STEP_PREFIX: &str = "step-";
/// Width of the zero-padded step counter.
const STEP_WIDTH: usize = 6;
/// Name of the single meta blob under a session slug.
const META_KEY: &str = "meta";

/// Conversation metadata — the single `meta` blob for a session slug.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMeta {
    pub agent: String,
    pub created_by: String,
    pub created_at: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub uptime_secs: u64,
}

/// One persisted step under `sessions/<slug>/step-<nnnnnn>`. Serialized
/// as JSON and discriminated by shape: `Compact` and `Event` carry
/// required tagged fields and fail fast, so `Entry` (the catch-all)
/// must be last in the `untagged` list.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum ConversationLine {
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

/// A trace entry persisted alongside messages.
///
/// Captures the *how* of agent execution (which tools ran, how long
/// they took, why the agent stopped, what it cost) — information that
/// doesn't fit in the message stream itself but is invaluable for
/// debugging.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EventLine {
    /// One round of tool calls dispatched by the model.
    ToolStart {
        calls: Vec<ToolCallTrace>,
        ts: String,
    },
    /// A single tool call completed. Correlated to `ToolStart` via `call_id`.
    ToolResult {
        call_id: String,
        duration_ms: u64,
        ts: String,
    },
    /// Agent run finished — final metadata and token usage.
    Done {
        model: String,
        iterations: usize,
        stop_reason: String,
        usage: Usage,
        ts: String,
    },
    /// User steered the agent mid-stream.
    UserSteered { content: String, ts: String },
}

/// Compact tool call info for [`EventLine::ToolStart`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallTrace {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub arguments: String,
}

impl EventLine {
    /// Build a trace entry from an [`AgentEvent`]. Returns `None` for events
    /// that don't carry useful trace information (deltas, internal markers,
    /// duplicates of what messages already capture).
    pub fn from_agent_event(event: &AgentEvent) -> Option<Self> {
        let ts = chrono::Utc::now().to_rfc3339();
        match event {
            AgentEvent::ToolCallsStart(calls) => Some(Self::ToolStart {
                calls: calls
                    .iter()
                    .map(|c| ToolCallTrace {
                        id: c.id.clone(),
                        name: c.function.name.to_string(),
                        arguments: c.function.arguments.clone(),
                    })
                    .collect(),
                ts,
            }),
            AgentEvent::ToolResult {
                call_id,
                duration_ms,
                ..
            } => Some(Self::ToolResult {
                call_id: call_id.clone(),
                duration_ms: *duration_ms,
                ts,
            }),
            AgentEvent::Done(resp) => Some(Self::Done {
                model: resp.model.clone(),
                iterations: resp.iterations,
                stop_reason: resp.stop_reason.to_string(),
                usage: sum_step_usage(&resp.steps),
                ts,
            }),
            AgentEvent::UserSteered { content } => Some(Self::UserSteered {
                content: content.clone(),
                ts,
            }),
            _ => None,
        }
    }
}

/// Sum token usage across all steps of an agent run.
fn sum_step_usage(steps: &[AgentStep]) -> Usage {
    steps.iter().fold(Usage::default(), |mut acc, step| {
        let u = &step.usage;
        acc.prompt_tokens += u.prompt_tokens;
        acc.completion_tokens += u.completion_tokens;
        acc.total_tokens += u.total_tokens;
        if let Some(v) = u.prompt_cache_hit_tokens {
            *acc.prompt_cache_hit_tokens.get_or_insert(0) += v;
        }
        if let Some(v) = u.prompt_cache_miss_tokens {
            *acc.prompt_cache_miss_tokens.get_or_insert(0) += v;
        }
        acc
    })
}

/// A compaction archive segment — a titled snapshot of past conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveSegment {
    /// Short title derived from the compact summary.
    pub title: String,
    /// The compact summary text.
    pub summary: String,
    /// When this segment was archived.
    pub archived_at: String,
}

/// A conversation tied to a specific agent.
#[derive(Debug, Clone)]
pub struct Conversation {
    /// Unique conversation identifier (monotonic counter, runtime-only).
    pub id: u64,
    /// Name of the agent this conversation is bound to.
    pub agent: String,
    /// Conversation history (the working context for the LLM).
    pub history: Vec<HistoryEntry>,
    /// Origin of this conversation (e.g. "user", "tg:12345").
    pub created_by: String,
    /// Conversation title (set by the `set_title` tool).
    pub title: String,
    /// Accumulated active time in seconds (persisted to meta).
    pub uptime_secs: u64,
    /// When this conversation was loaded/created in this process.
    pub created_at: Instant,
    /// Storage slug (`{agent}_{sender}_{seq}`). `None` until the first
    /// persistence call materializes the session directory.
    pub slug: Option<String>,
    /// Monotonic counter for the next `step-<nnnnnn>` key. Recovered
    /// from `list` on load; incremented in-memory on each append.
    pub next_step: u64,
}

impl Conversation {
    /// Create a new conversation with an empty history.
    pub fn new(id: u64, agent: impl Into<String>, created_by: impl Into<String>) -> Self {
        Self {
            id,
            agent: agent.into(),
            history: Vec::new(),
            created_by: created_by.into(),
            title: String::new(),
            uptime_secs: 0,
            created_at: Instant::now(),
            slug: None,
            next_step: 1,
        }
    }

    /// Ensure a session slug exists, minting one and persisting an
    /// initial meta blob on first call. No-op if the slug was already
    /// assigned (e.g. on a load path).
    pub fn ensure_slug(&mut self, storage: &impl Storage) {
        if self.slug.is_some() {
            return;
        }
        let agent_slug = sender_slug(&self.agent);
        let sender = sender_slug(&self.created_by);
        let prefix = format!("{SESSIONS_PREFIX}{agent_slug}_{sender}_");
        let seq = next_seq(storage, &prefix);
        let slug = format!("{agent_slug}_{sender}_{seq}");
        self.slug = Some(slug.clone());
        self.write_meta(storage);
    }

    /// Write the meta blob for this conversation (overwrites any
    /// existing one).
    pub fn write_meta(&self, storage: &impl Storage) {
        let Some(ref slug) = self.slug else {
            return;
        };
        let meta = ConversationMeta {
            agent: self.agent.clone(),
            created_by: self.created_by.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            title: self.title.clone(),
            uptime_secs: self.uptime_secs,
        };
        let Ok(json) = serde_json::to_vec(&meta) else {
            return;
        };
        if let Err(e) = storage.put(&meta_key(slug), &json) {
            tracing::warn!("failed to write conversation meta: {e}");
        }
    }

    /// Alias kept for legacy callers: rewrite meta is exactly
    /// overwriting the single meta blob.
    pub fn rewrite_meta(&self, storage: &impl Storage) {
        self.write_meta(storage);
    }

    /// Set the conversation title and rewrite meta. The slug stays
    /// stable — renaming is a metadata edit on the single meta blob.
    pub fn set_title(&mut self, storage: &impl Storage, title: &str) {
        self.title = title.to_string();
        self.write_meta(storage);
    }

    /// Append history entries as individual `step-<nnnnnn>` keys.
    /// Auto-injected entries are skipped so they don't pollute replay.
    pub fn append_messages(&mut self, storage: &impl Storage, entries: &[HistoryEntry]) {
        for entry in entries {
            if entry.auto_injected {
                continue;
            }
            self.append_line(storage, ConversationLine::Entry(entry.clone()));
        }
    }

    /// Append trace event entries. Events are persisted alongside
    /// messages and compact markers but are skipped when reconstructing
    /// the LLM working context (see [`Self::load_context`]).
    pub fn append_events(&mut self, storage: &impl Storage, events: &[EventLine]) {
        for event in events {
            self.append_line(storage, ConversationLine::Event(event.clone()));
        }
    }

    /// Append a compact marker. The marker doubles as an archive
    /// boundary: it stores the summary, a short title, and a timestamp.
    pub fn append_compact(&mut self, storage: &impl Storage, summary: &str) {
        let line = ConversationLine::Compact {
            compact: summary.to_string(),
            title: compact_title(summary),
            archived_at: chrono::Utc::now().to_rfc3339(),
        };
        self.append_line(storage, line);
    }

    fn append_line(&mut self, storage: &impl Storage, line: ConversationLine) {
        let Some(ref slug) = self.slug else {
            tracing::warn!("append called on conversation with no slug");
            return;
        };
        let Ok(bytes) = serde_json::to_vec(&line) else {
            return;
        };
        let key = step_key(slug, self.next_step);
        self.next_step += 1;
        if let Err(e) = storage.put(&key, &bytes) {
            tracing::warn!("failed to write conversation step {key}: {e}");
        }
    }

    /// Load the working context (meta + LLM history) for the given
    /// session slug. Replay starts from the last compact marker
    /// forward; if there is no compact marker, all entries are
    /// returned. Trace events are excluded from the returned history.
    pub fn load_context(
        storage: &impl Storage,
        slug: &str,
    ) -> anyhow::Result<(ConversationMeta, Vec<HistoryEntry>)> {
        let meta_bytes = storage
            .get(&meta_key(slug))?
            .ok_or_else(|| anyhow::anyhow!("session {slug} has no meta blob"))?;
        let meta: ConversationMeta = serde_json::from_slice(&meta_bytes)?;

        // `list` returns sorted keys; step_key uses zero-padding so
        // lexicographic order matches insertion order.
        let step_prefix = step_prefix(slug);
        let keys = storage.list(&step_prefix)?;

        // Two-pass: first decode every step and find the last compact
        // index; second pass folds from there forward.
        let mut lines: Vec<ConversationLine> = Vec::with_capacity(keys.len());
        let mut last_compact_idx: Option<usize> = None;
        for key in &keys {
            let Some(bytes) = storage.get(key)? else {
                continue;
            };
            match serde_json::from_slice::<ConversationLine>(&bytes) {
                Ok(line) => {
                    if matches!(line, ConversationLine::Compact { .. }) {
                        last_compact_idx = Some(lines.len());
                    }
                    lines.push(line);
                }
                Err(e) => tracing::warn!("skipping unparsable step {key}: {e}"),
            }
        }

        let start = last_compact_idx.unwrap_or(0);
        let mut entries = Vec::new();
        for (i, line) in lines[start..].iter().enumerate() {
            match line {
                ConversationLine::Compact { compact, .. }
                    if i == 0 && last_compact_idx.is_some() =>
                {
                    entries.push(HistoryEntry::user(compact));
                }
                ConversationLine::Entry(entry) => entries.push(entry.clone()),
                // Skip events and subsequent compacts.
                ConversationLine::Event(_) | ConversationLine::Compact { .. } => {}
            }
        }

        Ok((meta, entries))
    }

    /// Load all archive segments for a session slug. Each compact
    /// marker becomes an [`ArchiveSegment`] with title, summary, and
    /// timestamp. Segments come back in chronological order.
    pub fn load_archives(
        storage: &impl Storage,
        slug: &str,
    ) -> anyhow::Result<Vec<ArchiveSegment>> {
        let keys = storage.list(&step_prefix(slug))?;
        let mut archives = Vec::new();
        for key in keys {
            let Some(bytes) = storage.get(&key)? else {
                continue;
            };
            if let Ok(ConversationLine::Compact {
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
}

/// Find the latest session slug for an (agent, created_by) identity,
/// or `None` if no session exists.
pub fn find_latest_conversation(
    storage: &impl Storage,
    agent: &str,
    created_by: &str,
) -> Option<String> {
    let agent_slug = sender_slug(agent);
    let sender = sender_slug(created_by);
    let prefix = format!("{SESSIONS_PREFIX}{agent_slug}_{sender}_");
    let keys = storage.list(&prefix).ok()?;
    let mut best: Option<(u32, String)> = None;
    for key in keys {
        // key looks like `sessions/<agent>_<sender>_<seq>/meta` or
        // `sessions/<agent>_<sender>_<seq>/step-<n>`. Extract the slug
        // segment between the prefix and the next `/`.
        let rest = &key[prefix.len()..];
        let Some(slash) = rest.find('/') else {
            continue;
        };
        let seq_str = &rest[..slash];
        let Ok(seq) = seq_str.parse::<u32>() else {
            continue;
        };
        let slug = format!("{agent_slug}_{sender}_{seq}");
        if best.as_ref().is_none_or(|(best_seq, _)| seq > *best_seq) {
            best = Some((seq, slug));
        }
    }
    best.map(|(_, slug)| slug)
}

/// Recover the next `step-<nnnnnn>` counter for a session by finding
/// the highest existing step key under its prefix. Used on load so
/// subsequent appends don't collide with persisted steps.
pub fn next_step_counter(storage: &impl Storage, slug: &str) -> u64 {
    let prefix = step_prefix(slug);
    let keys = storage.list(&prefix).unwrap_or_default();
    let mut max = 0u64;
    for key in keys {
        let suffix = &key[prefix.len()..];
        if let Ok(n) = suffix.parse::<u64>()
            && n > max
        {
            max = n;
        }
    }
    max + 1
}

/// Next sequence number for session slugs under a given prefix. The
/// prefix is the full `sessions/<agent>_<sender>_` (trailing
/// underscore) — we take every key under it, parse the seq segment,
/// and return `max + 1`.
fn next_seq(storage: &impl Storage, prefix: &str) -> u32 {
    let keys = storage.list(prefix).unwrap_or_default();
    let mut max = 0u32;
    for key in keys {
        let rest = &key[prefix.len()..];
        let Some(slash) = rest.find('/') else {
            continue;
        };
        if let Ok(seq) = rest[..slash].parse::<u32>()
            && seq > max
        {
            max = seq;
        }
    }
    max + 1
}

/// Storage key for a session's meta blob.
fn meta_key(slug: &str) -> String {
    format!("{SESSIONS_PREFIX}{slug}/{META_KEY}")
}

/// Prefix for a session's step keys — used with `Storage::list`.
fn step_prefix(slug: &str) -> String {
    format!("{SESSIONS_PREFIX}{slug}/{STEP_PREFIX}")
}

/// Storage key for a specific step under a session.
fn step_key(slug: &str, step: u64) -> String {
    format!(
        "{SESSIONS_PREFIX}{slug}/{STEP_PREFIX}{step:0width$}",
        width = STEP_WIDTH
    )
}

/// Derive a short title from a compact summary.
fn compact_title(summary: &str) -> String {
    let end = summary
        .find(['.', '!', '?'])
        .map(|i| i + 1)
        .unwrap_or(summary.len())
        .min(60);
    let title = summary[..summary.floor_char_boundary(end)].trim();
    title.to_string()
}

/// Sanitize a string into a filesystem-safe slug.
pub fn sender_slug(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}
