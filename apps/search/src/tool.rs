//! Tool schema definitions for the search extension service.

use schemars::JsonSchema;
use serde::Deserialize;
use wcore::{
    agent::{AsTool, ToolDescription},
    protocol::ext::ToolDef,
};

#[derive(Deserialize, JsonSchema)]
pub struct WebSearch {
    /// The search query string.
    pub query: String,
    /// Maximum number of results to return (default: 10).
    pub max_results: Option<usize>,
}

impl ToolDescription for WebSearch {
    const DESCRIPTION: &'static str = "Search the web for information. Returns titles, URLs, and descriptions from multiple search engines.";
}

#[derive(Deserialize, JsonSchema)]
pub struct WebFetch {
    /// The URL to fetch content from.
    pub url: String,
}

impl ToolDescription for WebFetch {
    const DESCRIPTION: &'static str = "Fetch a web page and return its clean text content with scripts, styles, and navigation stripped.";
}

/// Build proto `ToolDef` messages for all search tools.
pub fn tool_defs() -> Vec<ToolDef> {
    [WebSearch::as_tool(), WebFetch::as_tool()]
        .into_iter()
        .map(|t| ToolDef {
            name: t.name.to_string(),
            description: t.description.to_string(),
            parameters: serde_json::to_vec(&t.parameters).expect("schema serialization"),
            strict: t.strict,
        })
        .collect()
}
