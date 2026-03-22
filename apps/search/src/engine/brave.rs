use crate::engine::urlencoded;
use crate::error::EngineError;
use crate::result::SearchResult;
use reqwest::Client;
use scraper::{Html, Selector};

/// Brave Search backend (independent index).
///
/// Brave renders results client-side via SvelteKit. The server response
/// includes HTML snippet fragments and/or embedded JSON data in `<script>`
/// tags. We attempt CSS selectors first, then fall back to JSON extraction.
pub struct Brave;

impl super::SearchEngine for Brave {
    async fn search(
        &self,
        query: &str,
        page: u32,
        client: &Client,
        user_agent: &str,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let url = format!(
            "https://search.brave.com/search?q={}&source=web&offset={}",
            urlencoded(query),
            page
        );

        let resp = client
            .get(&url)
            .header("User-Agent", user_agent)
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header("Accept-Language", "en-US,en;q=0.5")
            .header("Cookie", "safesearch=off")
            .send()
            .await?
            .error_for_status()?;

        let html = resp.text().await?;

        // Try CSS-based extraction first (works if Brave serves SSR snippets)
        let results = parse_html_results(&html);
        if !results.is_empty() {
            return Ok(results);
        }

        // Fall back to JSON extraction from embedded script data
        parse_json_results(&html)
    }
}

/// Try to parse results from HTML snippet elements.
fn parse_html_results(html: &str) -> Vec<SearchResult> {
    let document = Html::parse_document(html);

    let snippet_sel = match Selector::parse("div.snippet[data-type=\"web\"]") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let title_sel = match Selector::parse(".title") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let desc_sel = match Selector::parse(".snippet-description, .snippet-content") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let mut results = Vec::new();

    for element in document.select(&snippet_sel) {
        let title_el = match element.select(&title_sel).next() {
            Some(el) => el,
            None => continue,
        };

        let title = title_el.text().collect::<String>();

        // URL is in the parent <a> or in the snippet's first link
        let url = element
            .select(&Selector::parse("a[href]").unwrap())
            .next()
            .and_then(|a| a.value().attr("href"))
            .unwrap_or_default();

        // Skip internal brave links
        if url.is_empty() || url.starts_with('/') {
            continue;
        }

        let description = element
            .select(&desc_sel)
            .next()
            .map(|el| el.text().collect::<String>())
            .unwrap_or_default();

        if !title.is_empty() {
            results.push(SearchResult {
                title: title.trim().to_string(),
                url: url.to_string(),
                description: description.trim().to_string(),
                engines: vec!["brave".into()],
                score: 0.0,
            });
        }
    }

    results
}

/// Extract results from JSON data embedded in `<script>` tags.
/// Brave embeds page data as JSON that the SvelteKit client hydrates.
fn parse_json_results(html: &str) -> Result<Vec<SearchResult>, EngineError> {
    // Look for the data payload in script tags. Brave embeds it as
    // `const data = [...]` or in a JSON blob within <script> elements.
    let mut results = Vec::new();

    // Find JSON arrays that look like search result data
    for segment in html.split("<script") {
        let segment = match segment.split("</script>").next() {
            Some(s) => s,
            None => continue,
        };

        // Skip segments without result-like content
        if !segment.contains("\"url\"") || !segment.contains("\"title\"") {
            continue;
        }

        // Try to find JSON objects with title/url/description fields
        extract_results_from_text(segment, &mut results);
    }

    Ok(results)
}

/// Scan text for JSON-like objects containing search result fields.
fn extract_results_from_text(text: &str, results: &mut Vec<SearchResult>) {
    // Find potential JSON object boundaries containing result data
    let mut search_from = 0;
    while let Some(start) = text[search_from..].find("{\"title\"") {
        let abs_start = search_from + start;
        // Find matching closing brace (simple depth tracking)
        if let Some(obj_str) = extract_json_object(&text[abs_start..]) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(obj_str) {
                let title = value["title"].as_str().unwrap_or_default();
                let url = value["url"].as_str().unwrap_or_default();
                let desc = value["description"]
                    .as_str()
                    .or_else(|| value["snippet"].as_str())
                    .unwrap_or_default();

                if !title.is_empty()
                    && !url.is_empty()
                    && (url.starts_with("http://") || url.starts_with("https://"))
                {
                    results.push(SearchResult {
                        title: title.to_string(),
                        url: url.to_string(),
                        description: desc.to_string(),
                        engines: vec!["brave".into()],
                        score: 0.0,
                    });
                }
            }
        }
        search_from = abs_start + 1;
    }
}

/// Extract a JSON object string starting at the given position.
fn extract_json_object(text: &str) -> Option<&str> {
    if !text.starts_with('{') {
        return None;
    }

    let mut depth = 0;
    let mut in_string = false;
    let mut escape = false;

    for (i, ch) in text.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        match ch {
            '\\' if in_string => escape = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[..=i]);
                }
            }
            _ => {}
        }
    }

    None
}
