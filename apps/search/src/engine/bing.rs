use crate::engine::urlencoded;
use crate::error::EngineError;
use crate::result::SearchResult;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use percent_encoding::percent_decode_str;
use reqwest::Client;
use scraper::{Html, Selector};

/// Bing web search backend.
pub struct Bing;

impl super::SearchEngine for Bing {
    async fn search(
        &self,
        query: &str,
        page: u32,
        client: &Client,
        user_agent: &str,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let first = page * 10 + 1;
        let url = format!(
            "https://www.bing.com/search?q={}&first={}",
            urlencoded(query),
            first
        );

        let resp = client
            .get(&url)
            .header("User-Agent", user_agent)
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header("Accept-Language", "en-US,en;q=0.5")
            .send()
            .await?
            .error_for_status()?;

        let html = resp.text().await?;
        parse_results(&html)
    }
}

fn parse_results(html: &str) -> Result<Vec<SearchResult>, EngineError> {
    let document = Html::parse_document(html);

    let result_sel = Selector::parse("#b_results li.b_algo")
        .map_err(|e| EngineError::Parse(format!("{e:?}")))?;
    let title_sel = Selector::parse("h2 a").map_err(|e| EngineError::Parse(format!("{e:?}")))?;
    let desc_sel = Selector::parse("p").map_err(|e| EngineError::Parse(format!("{e:?}")))?;

    let mut results = Vec::new();

    for element in document.select(&result_sel) {
        let title_el = match element.select(&title_sel).next() {
            Some(el) => el,
            None => continue,
        };

        let title = title_el.text().collect::<String>();
        let raw_href = title_el.value().attr("href").unwrap_or_default();
        let url = normalize_bing_url(raw_href);

        let description = element
            .select(&desc_sel)
            .next()
            .map(|el| el.text().collect::<String>())
            .unwrap_or_default();

        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult {
                title: title.trim().to_string(),
                url,
                description: description.trim().to_string(),
                engines: vec!["bing".into()],
                score: 0.0,
            });
        }
    }

    Ok(results)
}

/// Bing wraps some URLs in a redirect like `https://www.bing.com/ck/a?...&u=a1<base64>...`.
/// Extract the real URL from the base64-encoded `u` parameter.
fn normalize_bing_url(href: &str) -> String {
    if !href.contains("bing.com/ck/a") {
        return href.to_string();
    }

    // Find the `u` parameter
    let query = match href.find('?') {
        Some(pos) => &href[pos + 1..],
        None => return href.to_string(),
    };

    for param in query.split('&') {
        if let Some(value) = param.strip_prefix("u=") {
            let decoded = percent_decode_str(value).decode_utf8_lossy();
            // Bing prepends "a1" before the base64 payload
            let payload = if decoded.starts_with("a1") {
                &decoded[2..]
            } else {
                &decoded
            };

            // Bing omits base64 padding — add it back
            let mut padded = payload.to_string();
            let rem = padded.len() % 4;
            if rem != 0 {
                for _ in 0..(4 - rem) {
                    padded.push('=');
                }
            }

            if let Ok(bytes) = STANDARD.decode(&padded) {
                if let Ok(url) = String::from_utf8(bytes) {
                    if url.starts_with("http://") || url.starts_with("https://") {
                        return url;
                    }
                }
            }
        }
    }

    href.to_string()
}
