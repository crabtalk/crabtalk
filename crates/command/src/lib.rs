pub use anyhow;
pub use command_codegen::command;
pub use wcore;

pub mod service;
pub use service::{
    Service, install, is_installed, render_service_template, uninstall, verbose_flag, view_logs,
};

#[cfg(feature = "mcp")]
pub use axum;
#[cfg(feature = "mcp")]
pub use service::{McpService, run_mcp};

/// Shared entry point: init tracing from `-v` count (falling back to `RUST_LOG`),
/// build a tokio runtime, run the given async closure, and exit on error.
pub fn run<F, Fut>(verbose: u8, f: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let level = if verbose > 0 {
        Some(match verbose {
            1 => tracing::Level::INFO,
            2 => tracing::Level::DEBUG,
            _ => tracing::Level::TRACE,
        })
    } else {
        std::env::var("RUST_LOG").ok().map(|v| {
            match v.rsplit('=').next().unwrap_or(&v).to_lowercase().as_str() {
                "trace" => tracing::Level::TRACE,
                "debug" => tracing::Level::DEBUG,
                "info" => tracing::Level::INFO,
                "error" => tracing::Level::ERROR,
                _ => tracing::Level::WARN,
            }
        })
    };
    if let Some(level) = level {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_max_level(level)
            .init();
    }

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Error: failed to create tokio runtime: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = rt.block_on(f()) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
