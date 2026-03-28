//! Crabtalk CLI binary entry point.

use anyhow::Result;
use clap::Parser;
use crabtalk::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    let cli = Cli::parse();

    let level = match cli.log_filter() {
        Some(f) => {
            // Set RUST_LOG so spawned child services inherit the same level.
            // SAFETY: called in main before spawning any threads.
            unsafe { std::env::set_var("RUST_LOG", f) };
            parse_level(f)
        }
        None => std::env::var("RUST_LOG")
            .ok()
            .map(|v| parse_level(&v))
            .unwrap_or(tracing::Level::WARN),
    };
    tracing_subscriber::fmt()
        .with_max_level(level)
        .without_time()
        .with_target(false)
        .init();

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
