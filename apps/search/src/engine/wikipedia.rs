use crate::engine::urlencoded;
use crate::error::EngineError;
use crate::result::SearchResult;
use reqwest::Client;

/// Wikipedia opensearch API backend.
pub struct Wikipedia;

impl super::SearchEngine for Wikipedia {
    async fn search(
        &self,
        query: &str,
        _page: u32,
        client: &Client,
        user_agent: &str,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let url = format!(
            "https://en.wikipedia.org/w/api.php?action=opensearch&search={}&limit=10&namespace=0&format=json",
            urlencoded(query)
        );

        let resp = client
            .get(&url)
            .header("User-Agent", user_agent)
            .send()
            .await?
            .error_for_status()?;

        let body: serde_json::Value = resp.json().await?;

        // Opensearch returns: [query, [titles], [descriptions], [urls]]
        let titles = body
            .get(1)
            .and_then(|v| v.as_array())
            .ok_or_else(|| EngineError::Parse("missing titles array".into()))?;
        let descriptions = body
            .get(2)
            .and_then(|v| v.as_array())
            .ok_or_else(|| EngineError::Parse("missing descriptions array".into()))?;
        let urls = body
            .get(3)
            .and_then(|v| v.as_array())
            .ok_or_else(|| EngineError::Parse("missing urls array".into()))?;

        let mut results = Vec::new();
        for i in 0..titles.len() {
            let title = titles[i].as_str().unwrap_or_default();
            let description = descriptions.get(i).and_then(|v| v.as_str()).unwrap_or("");
            let url = urls.get(i).and_then(|v| v.as_str()).unwrap_or("");

            if !url.is_empty() {
                results.push(SearchResult {
                    title: title.to_string(),
                    url: url.to_string(),
                    description: description.to_string(),
                    engines: vec!["wikipedia".into()],
                    score: 0.0,
                });
            }
        }

        Ok(results)
    }
}
