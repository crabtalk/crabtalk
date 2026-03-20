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

    /// Install and start the search MCP server as a system service.
    #[cfg(feature = "mcp")]
    Start,

    /// Stop and uninstall the search MCP server system service.
    #[cfg(feature = "mcp")]
    Stop,

    /// Run the search MCP server directly (used by launchd/systemd).
    #[cfg(feature = "mcp")]
    Run,

    /// View search service logs.
    ///
    /// Extra flags (e.g. `-f`, `-n 100`) are passed through to `tail`.
    /// Defaults to `-n 50` if no flags are given.
    #[cfg(feature = "mcp")]
    Logs {
        /// Arguments passed through to `tail` (e.g. `-f`, `-n 100`).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        tail_args: Vec<String>,
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
            #[cfg(feature = "mcp")]
            Command::Start => {
                cmd_start()?;
            }
            #[cfg(feature = "mcp")]
            Command::Stop => {
                cmd_stop()?;
            }
            #[cfg(feature = "mcp")]
            Command::Run => {
                cmd_run().await.map_err(|e| Error::Config(e.to_string()))?;
            }
            #[cfg(feature = "mcp")]
            Command::Logs { tail_args } => {
                wcore::service::logs("search", &tail_args)
                    .map_err(|e| Error::Config(e.to_string()))?;
            }
        }

        Ok(())
    }
}

#[cfg(feature = "mcp")]
fn cmd_start() -> Result<(), Error> {
    use crate::service;

    let binary = std::env::current_exe().map_err(|e| Error::Config(e.to_string()))?;
    let dummy = std::path::Path::new("");
    let params = service::ServiceParams {
        label: "ai.crabtalk.search",
        description: "Search MCP Server",
        subcommand: "",
        log_name: "search",
        binary: &binary,
        socket: dummy,
        config_path: dummy,
    };
    service::install_search(&params).map_err(|e: anyhow::Error| Error::Config(e.to_string()))?;
    Ok(())
}

#[cfg(feature = "mcp")]
fn cmd_stop() -> Result<(), Error> {
    use crate::service;

    let binary = std::env::current_exe().map_err(|e| Error::Config(e.to_string()))?;
    let dummy = std::path::Path::new("");
    let params = service::ServiceParams {
        label: "ai.crabtalk.search",
        description: "Search MCP Server",
        subcommand: "",
        log_name: "search",
        binary: &binary,
        socket: dummy,
        config_path: dummy,
    };
    service::uninstall(&params).map_err(|e| Error::Config(e.to_string()))?;

    let port_file = wcore::paths::RUN_DIR.join("search.port");
    let _ = std::fs::remove_file(&port_file);
    Ok(())
}

#[cfg(feature = "mcp")]
async fn cmd_run() -> Result<(), Box<dyn std::error::Error>> {
    use crate::mcp::SearchServer;
    use rmcp::transport::streamable_http_server::{
        StreamableHttpService, session::local::LocalSessionManager,
    };

    let config = Default::default();
    let service: StreamableHttpService<SearchServer, LocalSessionManager> =
        StreamableHttpService::new(|| Ok(SearchServer::new()), Default::default(), config);

    let router = axum::Router::new().nest_service("/mcp", service);
    let tcp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = tcp_listener.local_addr()?;

    // Write port file to ~/.crabtalk/run/search.port
    let run_dir = &*wcore::paths::RUN_DIR;
    std::fs::create_dir_all(run_dir)?;
    std::fs::write(run_dir.join("search.port"), addr.port().to_string())?;

    eprintln!("MCP server listening on {addr}");
    axum::serve(tcp_listener, router).await?;
    Ok(())
}
