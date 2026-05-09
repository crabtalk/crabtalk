pub mod config_cmd;
pub mod engines;
pub mod fetch;
pub mod search;

use crate::config::{Config, OutputFormat};
use crate::error::Error;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "crabtalk-search", version, about = "Meta search engine CLI")]
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

#[cfg(feature = "mcp")]
#[command::command(kind = "mcp", name = "search")]
pub struct Mcp;

#[cfg(feature = "mcp")]
impl command::McpService for Mcp {
    fn router(&self) -> axum::Router {
        use crate::mcp::SearchServer;
        use rmcp::transport::streamable_http_server::{
            StreamableHttpService, session::local::LocalSessionManager,
        };

        let config = Default::default();
        let service: StreamableHttpService<SearchServer, LocalSessionManager> =
            StreamableHttpService::new(|| Ok(SearchServer::new()), Default::default(), config);
        axum::Router::new().nest_service("/mcp", service)
    }
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

    /// MCP service management.
    #[cfg(feature = "mcp")]
    #[command(flatten)]
    Mcp(McpCommand),
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
            #[cfg(feature = "mcp")]
            Command::Mcp(action) => {
                Mcp.exec(action)
                    .await
                    .map_err(|e| Error::Config(e.to_string()))?;
            }
        }

        Ok(())
    }
}
