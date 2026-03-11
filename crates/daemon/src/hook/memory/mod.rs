//! Graph-based memory hook — owns LanceDB with entities, relations, and
//! journals tables. Registers `remember`, `recall`, `relate`, `connections`,
//! `compact`, and `distill` tool schemas. Journals store compaction summaries
//! with vector embeddings for semantic search via candle (all-MiniLM-L6-v2).

pub use config::MemoryConfig;
use embedder::Embedder;
use lance::LanceStore;
use std::path::Path;
use std::sync::Mutex;
use wcore::{AgentConfig, Hook, ToolRegistry, agent::AsTool, model::Tool, paths::CONFIG_DIR};

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
}

impl MemoryHook {
    /// Create a new MemoryHook, opening or creating the LanceDB database.
    pub async fn open(memory_dir: impl AsRef<Path>, config: &MemoryConfig) -> anyhow::Result<Self> {
        let memory_dir = memory_dir.as_ref();
        tokio::fs::create_dir_all(memory_dir).await?;
        let lance_dir = memory_dir.join("lance");
        let lance = LanceStore::open(&lance_dir).await?;

        let cache_dir = CONFIG_DIR.join(".cache").join("huggingface");
        let embedder = tokio::task::spawn_blocking(move || Embedder::load(&cache_dir)).await??;

        let allowed_entities = merge_defaults(DEFAULT_ENTITIES, &config.entities);
        let allowed_relations = merge_defaults(DEFAULT_RELATIONS, &config.relations);
        let connection_limit = config.connections.clamp(1, 100);

        Ok(Self {
            lance,
            embedder: Mutex::new(embedder),
            allowed_entities,
            allowed_relations,
            connection_limit,
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

                // Inject identity entities.
                if let Ok(identities) = lance.query_by_type(&agent_name, "identity", 50).await
                    && !identities.is_empty()
                {
                    buf.push_str("\n\n<identity>\n");
                    for e in &identities {
                        buf.push_str(&format!("- **{}**: {}\n", e.key, e.value));
                    }
                    buf.push_str("</identity>");
                }

                // Inject profile entities.
                if let Ok(profiles) = lance.query_by_type(&agent_name, "profile", 50).await
                    && !profiles.is_empty()
                {
                    buf.push_str("\n\n<profile>\n");
                    for e in &profiles {
                        buf.push_str(&format!("- **{}**: {}\n", e.key, e.value));
                    }
                    buf.push_str("</profile>");
                }

                // Inject recent journal entries.
                if let Ok(journals) = lance.recent_journals(&agent_name, 3).await
                    && !journals.is_empty()
                {
                    buf.push_str("\n\n<journal>\n");
                    for j in &journals {
                        let ts = chrono::DateTime::from_timestamp(j.created_at as i64, 0)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| j.created_at.to_string());
                        // Truncate summary to avoid bloating the system prompt.
                        let summary = if j.summary.len() > 500 {
                            format!("{}...", &j.summary[..500])
                        } else {
                            j.summary.clone()
                        };
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
