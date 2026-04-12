//! Crabtalk daemon binary entry point.

use anyhow::Result;
use clap::Parser;
use crabtalkd::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.foreground && cli.verbose > 0 {
        let level = match cli.verbose {
            1 => "crabtalk=info",
            2 => "crabtalk=debug",
            _ => "crabtalk=trace",
        };
        // SAFETY: called in main before spawning any threads.
        unsafe { std::env::set_var("RUST_LOG", level) };
        let level = parse_level(level);
        tracing_subscriber::fmt()
            .with_max_level(level)
            .without_time()
            .with_target(false)
            .init();
    } else if let Ok(val) = std::env::var("RUST_LOG") {
        let level = parse_level(&val);
        tracing_subscriber::fmt()
            .with_max_level(level)
            .without_time()
            .with_target(false)
            .init();
    }

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
