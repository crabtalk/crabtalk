//! Gateway CLI — clap command definitions.

use clap::Parser;

pub mod serve;

/// Walrus gateway service.
#[derive(Parser)]
#[command(name = "walrus-gateway")]
pub struct App {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand)]
pub enum Command {
    /// Run the gateway service, connecting to a walrus daemon.
    Serve {
        /// Daemon UDS socket path to connect to.
        #[arg(long)]
        daemon: String,
        /// JSON-encoded gateway config (telegram/discord tokens).
        #[arg(long, default_value = "{}")]
        config: String,
    },
}
