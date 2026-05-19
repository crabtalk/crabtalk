//! `crabup` binary entry point.

use clap::Parser;
use crabup::Cli;

fn main() {
    if let Err(e) = Cli::parse().run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
