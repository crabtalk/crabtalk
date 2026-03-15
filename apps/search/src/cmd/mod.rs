pub mod config_cmd;
pub mod engines;
pub mod fetch;
pub mod search;
pub mod serve;

use crate::config::{Config, OutputFormat};
use crate::error::Error;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "walrus-search", version, about = "Meta search engine CLI")]
pub struct App {
    /// Path to config file.
    #[arg(long, short, global = true)]
    pub config: Option<PathBuf>,

    /// Output format (json, text, compact). Overrides config.
    #[arg(long, short, global = true)]
    pub format: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Search across all configured engines.
    Search {
        /// The search query.
        query: String,

        /// Override engines (comma-separated, e.g. "wikipedia,duckduckgo").
        #[arg(long, short)]
        engines: Option<String>,

        /// Maximum number of results.
        #[arg(long, short = 'n')]
        max_results: Option<usize>,
    },

    /// List available search engines.
    Engines,

    /// Fetch a web page and extract clean text content.
    Fetch {
        /// The URL to fetch.
        url: String,
    },

    /// Show or generate configuration.
    Config {
        /// Print default config template to stdout.
        #[arg(long)]
        init: bool,
    },

    /// Run as a WHS hook service over a Unix domain socket.
    Serve {
        /// Path to the UDS socket to bind.
        #[arg(long)]
        socket: PathBuf,
    },
}

impl App {
    pub async fn run() -> Result<(), Error> {
        let app = App::parse();

        let config = match &app.config {
            Some(path) => Config::load(path)?,
            None => Config::discover(),
        };

        match app.command {
            Command::Search {
                query,
                engines,
                max_results,
            } => {
                search::run(query, engines, max_results, app.format, config).await?;
            }
            Command::Fetch { url } => {
                let format = app
                    .format
                    .as_deref()
                    .map(|f| match f {
                        "text" => OutputFormat::Text,
                        "compact" => OutputFormat::Compact,
                        _ => OutputFormat::Json,
                    })
                    .unwrap_or(config.output_format.clone());
                fetch::run(url, &format).await?;
            }
            Command::Engines => {
                engines::run();
            }
            Command::Config { init } => {
                config_cmd::run(&config, init);
            }
            Command::Serve { socket } => {
                serve::run(&socket)
                    .await
                    .map_err(|e| Error::Other(e.to_string()))?;
            }
        }

        Ok(())
    }
}
