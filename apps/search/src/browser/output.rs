use crate::config::OutputFormat;
use crate::result::SearchResults;

/// Format search results according to the chosen output format.
pub fn format_results(results: &SearchResults, format: &OutputFormat) -> String {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(results).unwrap_or_default(),
        OutputFormat::Text => format_text(results),
        OutputFormat::Compact => format_compact(results),
    }
}

fn format_text(results: &SearchResults) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Search: \"{}\" ({} results in {}ms)\n\n",
        results.query,
        results.results.len(),
        results.elapsed_ms
    ));

    for (i, r) in results.results.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", i + 1, r.title));
        out.push_str(&format!("   {}\n", r.url));
        if !r.description.is_empty() {
            out.push_str(&format!("   {}\n", r.description));
        }
        out.push_str(&format!("   [{}]\n\n", r.engines.join(", ")));
    }

    if !results.engine_errors.is_empty() {
        out.push_str("Errors:\n");
        for e in &results.engine_errors {
            out.push_str(&format!("  - {}: {}\n", e.engine, e.error));
        }
    }

    out
}

fn format_compact(results: &SearchResults) -> String {
    let mut out = String::new();
    for r in &results.results {
        out.push_str(&format!("{} | {}\n", r.title, r.url));
    }
    out
}
