//! Input parameters for the memory tools.

use schemars::JsonSchema;
use serde::Deserialize;
use wcore::{
    agent::{AsTool, ToolDescription},
    protocol::ext::ToolDef,
};

#[derive(Deserialize, JsonSchema)]
pub struct Recall {
    /// Batch of search queries to run against memory.
    pub queries: Vec<String>,
    /// Maximum number of results per query (default: 5).
    pub limit: Option<u32>,
}

impl ToolDescription for Recall {
    const DESCRIPTION: &'static str =
        "Search memory by one or more queries. Returns relevant entities and graph connections.";
}

#[derive(Deserialize, JsonSchema)]
pub struct Extract {
    /// Entities to upsert.
    pub entities: Vec<ExtractEntity>,
    /// Relations to upsert.
    pub relations: Vec<ExtractRelation>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ExtractEntity {
    /// Human-readable key/name for the entity.
    pub key: String,
    /// Value/content to store.
    pub value: String,
    /// Optional entity type (e.g. "fact", "person", "preference").
    pub entity_type: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ExtractRelation {
    /// Key of the source entity.
    pub source: String,
    /// Relation label (e.g. "knows", "prefers", "related_to").
    pub relation: String,
    /// Key of the target entity.
    pub target: String,
}

impl ToolDescription for Extract {
    const DESCRIPTION: &'static str =
        "Batch upsert entities and relations into memory. Internal tool for extraction.";
}

/// Build agent-visible tool defs: only `recall`.
pub fn tool_defs() -> Vec<ToolDef> {
    let t = Recall::as_tool();
    vec![ToolDef {
        name: t.name.to_string(),
        description: t.description.to_string(),
        parameters: serde_json::to_vec(&t.parameters).expect("schema serialization"),
        strict: t.strict,
    }]
}

/// Build all tool defs including internal ones (for extension RegisterTools response).
///
/// The daemon needs `extract` in the ToolSchemas so `infer_fulfill` can
/// provide it to the extraction LLM.
pub fn all_tool_defs() -> Vec<ToolDef> {
    [Recall::as_tool(), Extract::as_tool()]
        .into_iter()
        .map(|t| ToolDef {
            name: t.name.to_string(),
            description: t.description.to_string(),
            parameters: serde_json::to_vec(&t.parameters).expect("schema serialization"),
            strict: t.strict,
        })
        .collect()
}
