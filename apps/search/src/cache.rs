use crate::result::SearchResults;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

struct CacheEntry {
    results: SearchResults,
    inserted_at: Instant,
}

/// Simple in-memory cache with TTL.
pub struct Cache {
    entries: Mutex<HashMap<String, CacheEntry>>,
    ttl: Duration,
}

impl Cache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    pub fn get(&self, query: &str, page: u32) -> Option<SearchResults> {
        let key = Self::cache_key(query, page);
        let mut map = self.entries.lock().ok()?;

        if let Some(entry) = map.get(&key) {
            if entry.inserted_at.elapsed() < self.ttl {
                return Some(entry.results.clone());
            }
            // Expired — remove it
            map.remove(&key);
        }
        None
    }

    pub fn insert(&self, query: &str, page: u32, results: SearchResults) {
        let key = Self::cache_key(query, page);
        if let Ok(mut map) = self.entries.lock() {
            map.insert(
                key,
                CacheEntry {
                    results,
                    inserted_at: Instant::now(),
                },
            );
        }
    }

    fn cache_key(query: &str, page: u32) -> String {
        format!("{query}:{page}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result::SearchResults;

    fn dummy_results() -> SearchResults {
        SearchResults {
            query: "test".into(),
            results: vec![],
            engine_errors: vec![],
            elapsed_ms: 0,
        }
    }

    #[test]
    fn cache_hit_before_ttl() {
        let cache = Cache::new(60);
        cache.insert("test", 0, dummy_results());
        assert!(cache.get("test", 0).is_some());
    }

    #[test]
    fn cache_miss_different_key() {
        let cache = Cache::new(60);
        cache.insert("test", 0, dummy_results());
        assert!(cache.get("other", 0).is_none());
    }

    #[test]
    fn cache_miss_after_ttl() {
        let cache = Cache::new(0); // 0 second TTL
        cache.insert("test", 0, dummy_results());
        // TTL is 0 seconds, so elapsed >= ttl immediately
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(cache.get("test", 0).is_none());
    }
}
