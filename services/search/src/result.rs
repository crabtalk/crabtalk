use serde::{Deserialize, Serialize};

/// A single search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub description: String,
    pub engines: Vec<String>,
    pub score: f64,
}

/// Aggregated search results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResults {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub engine_errors: Vec<EngineErrorInfo>,
    pub elapsed_ms: u64,
}

/// Information about a failed engine query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineErrorInfo {
    pub engine: String,
    pub error: String,
}
