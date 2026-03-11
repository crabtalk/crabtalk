pub mod duckduckgo;
pub mod wikipedia;

use crate::error::EngineError;

/// URL-encode a string for use in query parameters.
pub fn urlencoded(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}
use crate::result::SearchResult;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Identifies a search engine backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineId {
    Wikipedia,
    DuckDuckGo,
}

impl EngineId {
    pub const ALL: &[EngineId] = &[EngineId::Wikipedia, EngineId::DuckDuckGo];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Wikipedia => "Wikipedia",
            Self::DuckDuckGo => "DuckDuckGo",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::Wikipedia => "Wikipedia opensearch API",
            Self::DuckDuckGo => "DuckDuckGo HTML scraper",
        }
    }
}

impl std::fmt::Display for EngineId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// The core trait for search engine backends.
pub trait SearchEngine: Send + Sync {
    fn search(
        &self,
        query: &str,
        page: u32,
        client: &Client,
        user_agent: &str,
    ) -> impl Future<Output = Result<Vec<SearchResult>, EngineError>> + Send;
}

/// Object-safe wrapper for dynamic dispatch.
pub trait SearchEngineDyn: Send + Sync {
    fn search_dyn<'a>(
        &'a self,
        query: &'a str,
        page: u32,
        client: &'a Client,
        user_agent: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SearchResult>, EngineError>> + Send + 'a>>;
}

impl<T: SearchEngine> SearchEngineDyn for T {
    fn search_dyn<'a>(
        &'a self,
        query: &'a str,
        page: u32,
        client: &'a Client,
        user_agent: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SearchResult>, EngineError>> + Send + 'a>> {
        Box::pin(self.search(query, page, client, user_agent))
    }
}

/// Registry mapping engine IDs to their implementations.
pub struct EngineRegistry {
    engines: Vec<(EngineId, Arc<dyn SearchEngineDyn>)>,
}

impl EngineRegistry {
    /// Build a registry with the given engine IDs.
    pub fn new(ids: &[EngineId]) -> Self {
        let engines = ids
            .iter()
            .map(|id| {
                let engine: Arc<dyn SearchEngineDyn> = match id {
                    EngineId::Wikipedia => Arc::new(wikipedia::Wikipedia),
                    EngineId::DuckDuckGo => Arc::new(duckduckgo::DuckDuckGo),
                };
                (*id, engine)
            })
            .collect();
        Self { engines }
    }

    pub fn engines(&self) -> &[(EngineId, Arc<dyn SearchEngineDyn>)] {
        &self.engines
    }
}
