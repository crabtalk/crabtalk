//! Walrus Discord gateway entry point.

use clap::Parser;
use walrus_discord::cmd::{App, Command};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let app = App::parse();
    match app.command {
        Command::Serve { daemon, config } => {
            walrus_discord::cmd::serve::run(&daemon, &config).await
        }
    }
}
