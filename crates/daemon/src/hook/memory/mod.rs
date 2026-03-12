//! Graph-based memory hook — owns LanceDB with entities, relations, and
//! journals tables. Registers `remember`, `recall`, `relate`, `connections`,
//! `compact`, and `distill` tool schemas. Journals store compaction summaries
//! with vector embeddings for semantic search via candle (all-MiniLM-L6-v2).

pub use config::MemoryConfig;
use embedder::Embedder;
use lance::LanceStore;
use std::path::Path;
use std::sync::Mutex;
use wcore::{
    AgentConfig, Hook, ToolRegistry,
    agent::AsTool,
    model::{Message, Role, Tool},
    paths::CONFIG_DIR,
};

pub mod config;
pub(crate) mod dispatch;
pub(crate) mod embedder;
pub(crate) mod lance;
pub(crate) mod tool;

const MEMORY_PROMPT: &str = include_str!("../../../prompts/memory.md");

/// Default entity types provided by the framework.
const DEFAULT_ENTITIES: &[&str] = &[
    "fact",
    "preference",
    "person",
    "event",
    "concept",
    "identity",
    "profile",
];

/// Default relation types provided by the framework.
const DEFAULT_RELATIONS: &[&str] = &[
    "knows",
    "prefers",
    "related_to",
    "caused_by",
    "part_of",
    "depends_on",
    "tagged_with",
];

/// Graph-based memory hook owning LanceDB entity, relation, and journal storage.
pub struct MemoryHook {
    pub(crate) lance: LanceStore,
    pub(crate) embedder: Mutex<Embedder>,
    pub(crate) allowed_entities: Vec<String>,
    pub(crate) allowed_relations: Vec<String>,
    pub(crate) connection_limit: usize,
    pub(crate) auto_recall: bool,
}

impl MemoryHook {
    /// Create a new MemoryHook, opening or creating the LanceDB database.
    pub async fn open(memory_dir: impl AsRef<Path>, config: &MemoryConfig) -> anyhow::Result<Self> {
        let memory_dir = memory_dir.as_ref();
        tokio::fs::create_dir_all(memory_dir).await?;

        // Load embedder first — needed for entity vector backfill during open.
        let cache_dir = CONFIG_DIR.join(".cache").join("huggingface");
        let embedder = tokio::task::spawn_blocking(move || Embedder::load(&cache_dir)).await??;

        let lance_dir = memory_dir.join("lance");
        let embed_mutex = Mutex::new(embedder);
        let lance = LanceStore::open(&lance_dir, |text| {
            let mut emb = embed_mutex
                .lock()
                .map_err(|e| anyhow::anyhow!("embedder lock poisoned: {e}"))?;
            emb.embed(text)
        })
        .await?;

        let allowed_entities = merge_defaults(DEFAULT_ENTITIES, &config.entities);
        let allowed_relations = merge_defaults(DEFAULT_RELATIONS, &config.relations);
        let connection_limit = config.connections.clamp(1, 100);

        Ok(Self {
            lance,
            embedder: embed_mutex,
            allowed_entities,
            allowed_relations,
            connection_limit,
            auto_recall: config.auto_recall,
        })
    }

    /// Check if an entity type is allowed.
    pub(crate) fn is_valid_entity(&self, entity_type: &str) -> bool {
        self.allowed_entities.iter().any(|t| t == entity_type)
    }

    /// Check if a relation type is allowed.
    pub(crate) fn is_valid_relation(&self, relation: &str) -> bool {
        self.allowed_relations.iter().any(|r| r == relation)
    }

    /// Generate an embedding vector for text. Runs candle inference in a blocking task.
    pub(crate) async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let text = text.to_owned();
        tokio::task::block_in_place(|| {
            let mut embedder = self
                .embedder
                .lock()
                .map_err(|e| anyhow::anyhow!("embedder lock poisoned: {e}"))?;
            embedder.embed(&text)
        })
    }
}

/// Truncate a string at a UTF-8 safe boundary, appending "..." if truncated.
fn truncate_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_owned();
    }
    // Walk backward from max_bytes to find a char boundary.
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

fn merge_defaults(defaults: &[&str], extras: &[String]) -> Vec<String> {
    let mut merged: Vec<String> = defaults.iter().map(|s| (*s).to_owned()).collect();
    for t in extras {
        if !merged.contains(t) {
            merged.push(t.clone());
        }
    }
    merged
}

impl Hook for MemoryHook {
    fn on_build_agent(&self, mut config: AgentConfig) -> AgentConfig {
        // Entity injection from LanceDB happens synchronously via a blocking
        // read. We use tokio::task::block_in_place to avoid deadlocks since
        // Hook::on_build_agent is not async.
        let agent_name = config.name.to_string();
        let lance = &self.lance;

        // Inject <self> block — agent's static birth identity from config.
        let mut self_block = String::from("\n\n<self>\n");
        self_block.push_str(&format!("name: {}\n", config.name));
        if !config.description.is_empty() {
            self_block.push_str(&format!("description: {}\n", config.description));
        }
        self_block.push_str("</self>");

        let extra = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut buf = self_block;

                // Inject identity entities (shared across all agents).
                if let Ok(identities) = lance.query_by_type("identity", 50).await
                    && !identities.is_empty()
                {
                    buf.push_str("\n\n<identity>\n");
                    for e in &identities {
                        buf.push_str(&format!("- **{}**: {}\n", e.key, e.value));
                    }
                    buf.push_str("</identity>");
                }

                // Inject profile entities (shared across all agents).
                if let Ok(profiles) = lance.query_by_type("profile", 50).await
                    && !profiles.is_empty()
                {
                    buf.push_str("\n\n<profile>\n");
                    for e in &profiles {
                        buf.push_str(&format!("- **{}**: {}\n", e.key, e.value));
                    }
                    buf.push_str("</profile>");
                }

                // Inject recent journal entries (agent-scoped).
                if let Ok(journals) = lance.recent_journals(&agent_name, 3).await
                    && !journals.is_empty()
                {
                    buf.push_str("\n\n<journal>\n");
                    for j in &journals {
                        let ts = chrono::DateTime::from_timestamp(j.created_at as i64, 0)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| j.created_at.to_string());
                        // Truncate summary to avoid bloating the system prompt.
                        let summary = truncate_utf8(&j.summary, 500);
                        buf.push_str(&format!("- **{ts}**: {summary}\n"));
                    }
                    buf.push_str("</journal>");
                }

                buf
            })
        });

        if !extra.is_empty() {
            config.system_prompt = format!("{}{extra}", config.system_prompt);
        }
        config.system_prompt = format!("{}\n\n{MEMORY_PROMPT}", config.system_prompt);
        config
    }

    fn on_before_run(&self, agent: &str, history: &[Message]) -> Vec<Message> {
        if !self.auto_recall {
            return Vec::new();
        }

        // Extract the last user message as the recall query.
        let query = match history.iter().rev().find(|m| m.role == Role::User) {
            Some(m) if m.content.len() >= 10 => &m.content,
            _ => return Vec::new(),
        };

        let lance = &self.lance;
        let agent = agent.to_owned();
        let query = query.clone();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut lines = Vec::new();

                // Embed the user message once; reuse for entities + journals.
                let vector = match self.embed(&query).await {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!("auto-recall embed failed: {e}");
                        return Vec::new();
                    }
                };

                // Semantic entity search.
                let entities = lance
                    .search_entities_semantic(&vector, None, 5)
                    .await
                    .unwrap_or_default();
                for e in &entities {
                    lines.push(format!("[{}] {}: {}", e.entity_type, e.key, e.value));
                }

                // 1-hop connections for top-3 matched entities.
                for e in entities.iter().take(3) {
                    if let Ok(rels) = lance
                        .find_connections(&e.id, None, lance::Direction::Both, 5)
                        .await
                    {
                        for r in &rels {
                            let line = format!("{} -[{}]-> {}", r.source, r.relation, r.target);
                            if !lines.contains(&line) {
                                lines.push(line);
                            }
                        }
                    }
                }

                // Semantic journal search (reuse same embedding vector).
                if let Ok(journals) = lance.search_journals(&vector, &agent, 2).await {
                    for j in &journals {
                        let ts = chrono::DateTime::from_timestamp(j.created_at as i64, 0)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| j.created_at.to_string());
                        let summary = truncate_utf8(&j.summary, 300);
                        lines.push(format!("[journal {ts}] {summary}"));
                    }
                }

                if lines.is_empty() {
                    return Vec::new();
                }

                let block = format!("<recall>\n{}\n</recall>", lines.join("\n"));
                vec![Message::user(block)]
            })
        })
    }

    fn on_compact(&self, _prompt: &mut String) {
        // This hook is unused. Identity context is passed directly in
        // Agent::compact() which inserts the agent's system_prompt (containing
        // <self>, <identity>, <profile>, <journal> blocks) as a user message
        // before conversation history.
    }

    async fn on_register_tools(&self, tools: &mut ToolRegistry) {
        // remember and relate have dynamic descriptions (inject allowed types).
        tools.insert(Tool {
            description: format!(
                "Store a memory entity. Types: {}.",
                self.allowed_entities.join(", ")
            )
            .into(),
            ..tool::Remember::as_tool()
        });
        tools.insert(tool::Recall::as_tool());
        tools.insert(Tool {
            description: format!(
                "Create a directed relation between two entities by key. Relations: {}.",
                self.allowed_relations.join(", ")
            )
            .into(),
            ..tool::Relate::as_tool()
        });
        tools.insert(tool::Connections::as_tool());
        tools.insert(tool::Compact::as_tool());
        tools.insert(tool::Distill::as_tool());
    }
}
