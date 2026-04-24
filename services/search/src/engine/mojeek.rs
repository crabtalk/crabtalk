use crate::engine::urlencoded;
use crate::error::EngineError;
use crate::result::SearchResult;
use reqwest::Client;
use scraper::{Html, Selector};

/// Mojeek search engine backend (independent crawler-based index).
pub struct Mojeek;

impl super::SearchEngine for Mojeek {
    async fn search(
        &self,
        query: &str,
        page: u32,
        client: &Client,
        user_agent: &str,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let offset = page * 10 + 1;
        let url = format!(
            "https://www.mojeek.com/search?q={}&s={}",
            urlencoded(query),
            offset
        );

        let resp = client
            .get(&url)
            .header("User-Agent", user_agent)
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header("Accept-Language", "en-US,en;q=0.5")
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "none")
            .header("Upgrade-Insecure-Requests", "1")
            .send()
            .await?
            .error_for_status()?;

        let html = resp.text().await?;
        parse_results(&html)
    }
}

fn parse_results(html: &str) -> Result<Vec<SearchResult>, EngineError> {
    let document = Html::parse_document(html);

    let result_sel = Selector::parse("ul.results-standard li")
        .map_err(|e| EngineError::Parse(format!("{e:?}")))?;
    let title_sel = Selector::parse("a.title").map_err(|e| EngineError::Parse(format!("{e:?}")))?;
    let desc_sel = Selector::parse("p.s").map_err(|e| EngineError::Parse(format!("{e:?}")))?;

    let mut results = Vec::new();

    for element in document.select(&result_sel) {
        let title_el = match element.select(&title_sel).next() {
            Some(el) => el,
            None => continue,
        };

        let title = title_el.text().collect::<String>();
        let url = title_el.value().attr("href").unwrap_or_default();
        let description = element
            .select(&desc_sel)
            .next()
            .map(|el| el.text().collect::<String>())
            .unwrap_or_default();

        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult {
                title: title.trim().to_string(),
                url: url.to_string(),
                description: description.trim().to_string(),
                engines: vec!["mojeek".into()],
                score: 0.0,
            });
        }
    }

    Ok(results)
}
