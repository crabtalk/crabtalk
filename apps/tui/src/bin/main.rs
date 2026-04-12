//! Crabtalk TUI binary entry point.

use anyhow::Result;
use clap::Parser;
use crabtalk_tui::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    if let Ok(val) = std::env::var("RUST_LOG") {
        let level = parse_level(&val);
        tracing_subscriber::fmt()
            .with_max_level(level)
            .without_time()
            .with_target(false)
            .init();
    }

    let cli = Cli::parse();
    cli.run().await
}

/// Extract the most specific level from a filter string like "crabtalk=debug".
fn parse_level(s: &str) -> tracing::Level {
    let level_str = s.rsplit('=').next().unwrap_or(s);
    match level_str.to_lowercase().as_str() {
        "trace" => tracing::Level::TRACE,
        "debug" => tracing::Level::DEBUG,
        "info" => tracing::Level::INFO,
        "error" => tracing::Level::ERROR,
        _ => tracing::Level::WARN,
    }
}
