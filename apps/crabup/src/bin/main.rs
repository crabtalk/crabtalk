//! `crabup` binary entry point.

use clap::Parser;
use crabup::Cli;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(e) = Cli::parse().run().await {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
