use crate::browser::user_agent;
use crate::cache::Cache;
use crate::config::Config;
use crate::engine::EngineId;
use crate::engine::EngineRegistry;
use crate::error::Error;
use crate::result::{EngineErrorInfo, SearchResult, SearchResults};
use reqwest::Client;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;

/// The core aggregator that dispatches queries to engines and merges results.
pub struct Aggregator {
    registry: EngineRegistry,
    client: Client,
    config: Config,
    cache: Cache,
}

impl Aggregator {
    pub fn new(config: Config) -> Result<Self, Error> {
        if config.engines.is_empty() {
            return Err(Error::NoEngines);
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()?;

        let cache = Cache::new(config.cache_ttl_secs);
        let registry = EngineRegistry::new(&config.engines);

        Ok(Self {
            registry,
            client,
            config,
            cache,
        })
    }

    /// Search across all configured engines, returning merged and ranked results.
    pub async fn search(&self, query: &str, page: u32) -> Result<SearchResults, Error> {
        // Check cache
        if let Some(cached) = self.cache.get(query, page) {
            return Ok(cached);
        }

        let start = Instant::now();
        let ua = user_agent::random();

        // Dispatch to all engines in parallel
        let mut tasks: JoinSet<(EngineId, Result<Vec<SearchResult>, String>)> = JoinSet::new();

        let client = self.client.clone();

        for (id, engine) in self.registry.engines() {
            let id = *id;
            let query = query.to_string();
            let ua = ua.to_string();
            let client = client.clone();
            let engine = engine.clone();

            tasks.spawn(async move {
                let result = engine
                    .search_dyn(&query, page, &client, &ua)
                    .await
                    .map_err(|e| e.to_string());
                (id, result)
            });
        }

        let mut all_results: Vec<(EngineId, Vec<SearchResult>)> = Vec::new();
        let mut engine_errors: Vec<EngineErrorInfo> = Vec::new();

        while let Some(result) = tasks.join_next().await {
            match result {
                Ok((id, Ok(results))) => {
                    tracing::debug!(engine = %id, count = results.len(), "engine returned results");
                    all_results.push((id, results));
                }
                Ok((id, Err(err))) => {
                    tracing::warn!(engine = %id, error = %err, "engine failed");
                    engine_errors.push(EngineErrorInfo {
                        engine: id.name().to_string(),
                        error: err,
                    });
                }
                Err(join_err) => {
                    tracing::error!(error = %join_err, "task panicked");
                }
            }
        }

        // Merge and deduplicate by URL
        let results = merge_and_rank(all_results, self.config.max_results);

        let elapsed_ms = start.elapsed().as_millis() as u64;
        let search_results = SearchResults {
            query: query.to_string(),
            results,
            engine_errors,
            elapsed_ms,
        };

        // Cache the results
        self.cache.insert(query, page, search_results.clone());

        Ok(search_results)
    }
}

/// Merge results from multiple engines, deduplicate by URL, apply consensus ranking.
fn merge_and_rank(
    engine_results: Vec<(EngineId, Vec<SearchResult>)>,
    max_results: usize,
) -> Vec<SearchResult> {
    // URL -> merged result
    let mut url_map: HashMap<String, SearchResult> = HashMap::new();
    // Track position-based scores: earlier positions get higher scores
    let mut url_position_scores: HashMap<String, f64> = HashMap::new();

    for (_engine_id, results) in &engine_results {
        for (position, result) in results.iter().enumerate() {
            let normalized = normalize_url(&result.url);

            // Position score: first result = 1.0, decaying
            let position_score = 1.0 / (position as f64 + 1.0);

            url_position_scores
                .entry(normalized.clone())
                .and_modify(|s| *s += position_score)
                .or_insert(position_score);

            url_map
                .entry(normalized)
                .and_modify(|existing| {
                    // Merge engine names
                    for engine in &result.engines {
                        if !existing.engines.contains(engine) {
                            existing.engines.push(engine.clone());
                        }
                    }
                    // Keep the longer description
                    if result.description.len() > existing.description.len() {
                        existing.description = result.description.clone();
                    }
                })
                .or_insert_with(|| result.clone());
        }
    }

    // Apply consensus scoring
    let mut results: Vec<SearchResult> = url_map
        .into_iter()
        .map(|(url, mut result)| {
            let engine_count = result.engines.len() as f64;
            let position_score = url_position_scores.get(&url).copied().unwrap_or(0.0);

            // Consensus bonus: 0.5 per additional engine
            let consensus_bonus = (engine_count - 1.0) * 0.5;
            result.score = position_score + consensus_bonus;
            result
        })
        .collect();

    // Sort by score descending
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    results.truncate(max_results);
    results
}

/// Normalize a URL for deduplication.
fn normalize_url(raw: &str) -> String {
    let mut s = raw.trim().to_string();

    // Strip trailing slash
    if s.ends_with('/') {
        s.pop();
    }

    // Normalize www prefix
    s = s.replace("://www.", "://");

    // Strip common tracking params
    if let Some(pos) = s.find('?') {
        let base = &s[..pos];
        let query = &s[pos + 1..];
        let filtered: Vec<&str> = query
            .split('&')
            .filter(|param| {
                let key = param.split('=').next().unwrap_or("");
                !key.starts_with("utm_") && key != "fbclid" && key != "gclid" && key != "ref"
            })
            .collect();
        if filtered.is_empty() {
            s = base.to_string();
        } else {
            s = format!("{}?{}", base, filtered.join("&"));
        }
    }

    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_tracking_params() {
        assert_eq!(
            normalize_url("https://example.com/page?utm_source=google&id=1"),
            "https://example.com/page?id=1"
        );
    }

    #[test]
    fn normalize_strips_www() {
        assert_eq!(
            normalize_url("https://www.example.com/page"),
            "https://example.com/page"
        );
    }

    #[test]
    fn normalize_strips_trailing_slash() {
        assert_eq!(
            normalize_url("https://example.com/page/"),
            "https://example.com/page"
        );
    }

    #[test]
    fn merge_deduplicates_and_ranks() {
        let engine_results = vec![
            (
                EngineId::Wikipedia,
                vec![
                    SearchResult {
                        title: "Rust".into(),
                        url: "https://en.wikipedia.org/wiki/Rust".into(),
                        description: "A language".into(),
                        engines: vec!["wikipedia".into()],
                        score: 0.0,
                    },
                    SearchResult {
                        title: "Shared".into(),
                        url: "https://example.com/shared".into(),
                        description: "Short".into(),
                        engines: vec!["wikipedia".into()],
                        score: 0.0,
                    },
                ],
            ),
            (
                EngineId::DuckDuckGo,
                vec![SearchResult {
                    title: "Shared Result".into(),
                    url: "https://example.com/shared".into(),
                    description: "A longer description".into(),
                    engines: vec!["duckduckgo".into()],
                    score: 0.0,
                }],
            ),
        ];

        let results = merge_and_rank(engine_results, 10);

        // The shared URL should be first (appears in both engines)
        assert_eq!(results[0].url, "https://example.com/shared");
        assert_eq!(results[0].engines.len(), 2);
        // Should keep the longer description
        assert_eq!(results[0].description, "A longer description");
        // Should have 2 total results
        assert_eq!(results.len(), 2);
    }
}
