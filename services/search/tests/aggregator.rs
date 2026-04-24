use crabtalk_search::aggregator::{merge_and_rank, normalize_url};
use crabtalk_search::engine::EngineId;
use crabtalk_search::result::SearchResult;

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
