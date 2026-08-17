//! Crabtalk daemon binary entry point.

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let level = std::env::var("RUST_LOG")
        .map(|val| parse_level(&val))
        .unwrap_or(tracing::Level::INFO);
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(level)
        .init();

    crabtalkd::foreground::start().await
}

/// Extract the most specific level from a filter string like "crabtalk=debug".
fn parse_level(s: &str) -> tracing::Level {
    let level_str = s.rsplit('=').next().unwrap_or(s);
    match level_str.to_lowercase().as_str() {
        "trace" => tracing::Level::TRACE,
        "debug" => tracing::Level::DEBUG,
        "warn" => tracing::Level::WARN,
        "error" => tracing::Level::ERROR,
        _ => tracing::Level::INFO,
    }
}
