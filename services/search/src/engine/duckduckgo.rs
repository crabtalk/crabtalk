use crate::engine::urlencoded;
use crate::error::EngineError;
use crate::result::SearchResult;
use percent_encoding::percent_decode_str;
use reqwest::Client;
use scraper::{Html, Selector};

/// DuckDuckGo HTML scraping backend.
pub struct DuckDuckGo;

impl super::SearchEngine for DuckDuckGo {
    async fn search(
        &self,
        query: &str,
        _page: u32,
        client: &Client,
        user_agent: &str,
    ) -> Result<Vec<SearchResult>, EngineError> {
        // Use the lite endpoint — less aggressive bot detection than html/
        let resp = client
            .get("https://lite.duckduckgo.com/lite/")
            .header("User-Agent", user_agent)
            .header("Referer", "https://lite.duckduckgo.com/")
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header("Accept-Language", "en-US,en;q=0.5")
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "same-origin")
            .header("Upgrade-Insecure-Requests", "1")
            .query(&[("q", query)])
            .send()
            .await?
            .error_for_status()?;

        let html = resp.text().await?;
        let results = parse_lite_results(&html);

        // Fall back to html/ endpoint if lite returned nothing
        if results.is_empty() {
            let resp = client
                .post("https://html.duckduckgo.com/html/")
                .header("User-Agent", user_agent)
                .header("Referer", "https://html.duckduckgo.com/")
                .header(
                    "Accept",
                    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
                )
                .header("Accept-Language", "en-US,en;q=0.5")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(format!("q={}&b=", urlencoded(query)))
                .send()
                .await?
                .error_for_status()?;

            let html = resp.text().await?;
            return parse_html_results(&html);
        }

        Ok(results)
    }
}

/// Parse results from the DDG lite endpoint.
/// The lite page uses a table layout: each result is a group of <tr> rows
/// with class "result-link" (title+url), "result-snippet" (description).
fn parse_lite_results(html: &str) -> Vec<SearchResult> {
    let document = Html::parse_document(html);

    // Lite results: links are in <a class="result-link"> inside <td>
    let link_sel = Selector::parse("a.result-link").unwrap();
    // Snippets follow in a <td class="result-snippet">
    let snippet_sel = Selector::parse("td.result-snippet").unwrap();

    let links: Vec<_> = document.select(&link_sel).collect();
    let snippets: Vec<_> = document.select(&snippet_sel).collect();

    let mut results = Vec::new();
    for (i, link_el) in links.iter().enumerate() {
        let title = link_el.text().collect::<String>();
        let href = link_el.value().attr("href").unwrap_or_default();
        let url = normalize_ddg_url(href, "");
        let description = snippets
            .get(i)
            .map(|el| el.text().collect::<String>())
            .unwrap_or_default();

        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult {
                title: title.trim().to_string(),
                url,
                description: description.trim().to_string(),
                engines: vec!["duckduckgo".into()],
                score: 0.0,
            });
        }
    }

    results
}

/// Parse results from the DDG html/ endpoint (fallback).
fn parse_html_results(html: &str) -> Result<Vec<SearchResult>, EngineError> {
    let document = Html::parse_document(html);

    let result_sel =
        Selector::parse(".result").map_err(|e| EngineError::Parse(format!("{e:?}")))?;
    let title_sel =
        Selector::parse(".result__a").map_err(|e| EngineError::Parse(format!("{e:?}")))?;
    let snippet_sel =
        Selector::parse(".result__snippet").map_err(|e| EngineError::Parse(format!("{e:?}")))?;
    let url_sel =
        Selector::parse(".result__url").map_err(|e| EngineError::Parse(format!("{e:?}")))?;

    let mut results = Vec::new();

    for element in document.select(&result_sel) {
        let title = element
            .select(&title_sel)
            .next()
            .map(|el| el.text().collect::<String>())
            .unwrap_or_default();

        let snippet = element
            .select(&snippet_sel)
            .next()
            .map(|el| el.text().collect::<String>())
            .unwrap_or_default();

        let raw_url = element
            .select(&url_sel)
            .next()
            .map(|el| el.text().collect::<String>())
            .unwrap_or_default();

        let href_url = element
            .select(&title_sel)
            .next()
            .and_then(|el| el.value().attr("href"))
            .unwrap_or_default();

        let url = normalize_ddg_url(href_url, &raw_url);

        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult {
                title: title.trim().to_string(),
                url,
                description: snippet.trim().to_string(),
                engines: vec!["duckduckgo".into()],
                score: 0.0,
            });
        }
    }

    Ok(results)
}

/// Normalize DuckDuckGo's URL format.
/// DDG sometimes wraps URLs in redirect links or shows them without scheme.
fn normalize_ddg_url(href: &str, text_url: &str) -> String {
    // Try to extract from DDG redirect URL
    if let Some(pos) = href.find("uddg=") {
        let encoded = &href[pos + 5..];
        let end = encoded.find('&').unwrap_or(encoded.len());
        let decoded = percent_decode_str(&encoded[..end])
            .decode_utf8_lossy()
            .to_string();
        if !decoded.is_empty() {
            return decoded;
        }
    }

    // If href looks like a normal URL, use it
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }

    // Fall back to the text URL, adding scheme if needed
    let trimmed = text_url.trim();
    if !trimmed.is_empty() {
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            return trimmed.to_string();
        }
        return format!("https://{trimmed}");
    }

    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_ddg_url_with_redirect() {
        let href = "/l/?uddg=https%3A%2F%2Fexample.com%2Fpage&rut=abc";
        assert_eq!(normalize_ddg_url(href, ""), "https://example.com/page");
    }

    #[test]
    fn normalize_ddg_url_plain_text() {
        assert_eq!(
            normalize_ddg_url("", "example.com/page"),
            "https://example.com/page"
        );
    }

    #[test]
    fn normalize_ddg_url_direct_href() {
        assert_eq!(
            normalize_ddg_url("https://example.com", ""),
            "https://example.com"
        );
    }
}
