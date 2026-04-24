use crabtalk_outlook::cmd::App;

#[tokio::main]
async fn main() {
    if let Some(level) = std::env::var("RUST_LOG").ok().map(|v| {
        match v.rsplit('=').next().unwrap_or(&v).to_lowercase().as_str() {
            "trace" => tracing::Level::TRACE,
            "debug" => tracing::Level::DEBUG,
            "info" => tracing::Level::INFO,
            "error" => tracing::Level::ERROR,
            _ => tracing::Level::WARN,
        }
    }) {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_max_level(level)
            .init();
    }

    if let Err(e) = App::run().await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
