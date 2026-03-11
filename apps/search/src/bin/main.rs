use walrus_search::cmd::App;

#[tokio::main]
async fn main() {
    if std::env::var_os("RUST_LOG").is_some() {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();
    }

    if let Err(e) = App::run().await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
