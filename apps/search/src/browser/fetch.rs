use crate::browser::user_agent;
use crate::error::Error;
use reqwest::Client;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Result of fetching and extracting content from a URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchResult {
    pub url: String,
    pub title: String,
    pub content: String,
    pub content_length: usize,
}

/// Fetch a URL and extract clean text content.
pub async fn fetch_url(url: &str, client: &Client) -> Result<FetchResult, Error> {
    let ua = user_agent::random();

    let resp = client
        .get(url)
        .header("User-Agent", ua)
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header("Accept-Language", "en-US,en;q=0.5")
        .header("Sec-Fetch-Dest", "document")
        .header("Sec-Fetch-Mode", "navigate")
        .header("Upgrade-Insecure-Requests", "1")
        .send()
        .await?
        .error_for_status()?;

    let final_url = resp.url().to_string();
    let html = resp.text().await?;

    let (title, content) = extract_content(&html);
    let content_length = content.len();

    Ok(FetchResult {
        url: final_url,
        title,
        content,
        content_length,
    })
}

/// Build a default client for fetch operations.
pub fn default_client() -> Result<Client, Error> {
    Ok(Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?)
}

/// Tags whose entire subtree should be removed before text extraction.
const NOISE_TAGS: &[&str] = &[
    "script", "style", "nav", "footer", "header", "aside", "noscript", "svg", "iframe", "form",
];

/// Extract title and clean text content from HTML.
fn extract_content(html: &str) -> (String, String) {
    let document = Html::parse_document(html);

    // Extract <title>
    let title = Selector::parse("title")
        .ok()
        .and_then(|sel| document.select(&sel).next())
        .map(|el| el.text().collect::<String>())
        .unwrap_or_default()
        .trim()
        .to_string();

    // Walk text nodes, skipping noise element subtrees
    let mut text_parts: Vec<String> = Vec::new();
    let body_sel = Selector::parse("body").ok();
    let root = body_sel
        .as_ref()
        .and_then(|sel| document.select(sel).next());

    if let Some(body) = root {
        collect_text(&body, &mut text_parts);
    } else {
        // No <body>, walk the whole document
        for node in document.tree.nodes() {
            if let scraper::node::Node::Text(text) = node.value() {
                let trimmed = text.text.trim();
                if !trimmed.is_empty() {
                    text_parts.push(trimmed.to_string());
                }
            }
        }
    }

    let content = normalize_whitespace(&text_parts.join("\n"));
    (title, content)
}

/// Recursively collect text from an element, skipping noise subtrees by tag name.
fn collect_text(element: &scraper::ElementRef, out: &mut Vec<String>) {
    for child in element.children() {
        match child.value() {
            scraper::node::Node::Text(text) => {
                let trimmed = text.text.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.to_string());
                }
            }
            scraper::node::Node::Element(_) => {
                if let Some(child_el) = scraper::ElementRef::wrap(child) {
                    let tag = child_el.value().name();

                    // Skip noise elements entirely
                    if NOISE_TAGS.contains(&tag) {
                        continue;
                    }

                    // Block-level elements get a blank line before them
                    if is_block_element(tag) && !out.is_empty() {
                        out.push(String::new());
                    }

                    collect_text(&child_el, out);
                }
            }
            _ => {}
        }
    }
}

/// Collapse multiple blank lines into at most one.
fn normalize_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_blank = false;

    for line in s.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !prev_blank && !result.is_empty() {
                result.push('\n');
                prev_blank = true;
            }
        } else {
            if prev_blank {
                result.push('\n');
            }
            if !result.is_empty() && !prev_blank {
                result.push('\n');
            }
            result.push_str(trimmed);
            prev_blank = false;
        }
    }

    result
}

fn is_block_element(tag: &str) -> bool {
    matches!(
        tag,
        "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "section" | "article" | "main"
            | "blockquote" | "pre" | "ul" | "ol" | "li" | "table" | "tr" | "br" | "hr" | "dl"
            | "dd" | "dt" | "figcaption" | "figure"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_title_and_body_text() {
        let html = r#"
        <html>
        <head><title>Test Page</title></head>
        <body>
            <h1>Hello World</h1>
            <p>This is a test paragraph.</p>
        </body>
        </html>
        "#;

        let (title, content) = extract_content(html);
        assert_eq!(title, "Test Page");
        assert!(content.contains("Hello World"));
        assert!(content.contains("This is a test paragraph."));
    }

    #[test]
    fn strips_script_and_style() {
        let html = r#"
        <html>
        <head><title>Clean</title><style>body { color: red; }</style></head>
        <body>
            <script>alert('xss')</script>
            <p>Visible content</p>
            <style>.hidden { display: none; }</style>
        </body>
        </html>
        "#;

        let (_, content) = extract_content(html);
        assert!(!content.contains("alert"));
        assert!(!content.contains("color: red"));
        assert!(!content.contains("display: none"));
        assert!(content.contains("Visible content"));
    }

    #[test]
    fn strips_nav_footer_header() {
        let html = r#"
        <html>
        <body>
            <header>Site Header</header>
            <nav>Navigation Links</nav>
            <main><p>Main content here</p></main>
            <footer>Copyright 2024</footer>
        </body>
        </html>
        "#;

        let (_, content) = extract_content(html);
        assert!(!content.contains("Site Header"));
        assert!(!content.contains("Navigation Links"));
        assert!(!content.contains("Copyright 2024"));
        assert!(content.contains("Main content here"));
    }

    #[test]
    fn normalize_collapses_blank_lines() {
        let input = "line1\n\n\n\nline2\n\n\nline3";
        let result = normalize_whitespace(input);
        assert_eq!(result, "line1\n\nline2\n\nline3");
    }
}
