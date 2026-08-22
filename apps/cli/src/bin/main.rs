//! The Crabtalk command line.

use clap::Parser;
use crabtalk_cli::cmd::Cli;

#[tokio::main]
async fn main() {
    if let Err(e) = Cli::parse().run().await {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
