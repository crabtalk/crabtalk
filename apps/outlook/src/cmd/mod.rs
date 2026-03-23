use crate::{auth, config::Config, error::Error, token::token_path};
use clap::{Parser, Subcommand};

#[cfg(feature = "mcp")]
#[crabtalk_command::command(kind = "mcp", name = "outlook")]
struct Outlook;

#[cfg(feature = "mcp")]
impl crabtalk_command::McpService for Outlook {
    fn router(&self) -> axum::Router {
        use crate::mcp::OutlookServer;
        use rmcp::transport::streamable_http_server::{
            StreamableHttpService, session::local::LocalSessionManager,
        };

        let config = Default::default();
        let service: StreamableHttpService<OutlookServer, LocalSessionManager> =
            StreamableHttpService::new(|| Ok(OutlookServer::new()), Default::default(), config);
        axum::Router::new().nest_service("/mcp", service)
    }
}

#[derive(Parser, Debug)]
#[command(name = "crabtalk-outlook", version, about = "Outlook MCP server")]
pub struct App {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Authenticate with Microsoft via browser login.
    Auth,

    /// MCP service management.
    #[cfg(feature = "mcp")]
    #[command(flatten)]
    Mcp(OutlookCommand),
}

impl App {
    pub async fn run() -> Result<(), Error> {
        let app = App::parse();
        match app.command {
            Command::Auth => {
                let config = Config::load()?;
                let token = auth::authorize(&config).await?;
                let path = token_path();
                token.save(&path)?;
                eprintln!("Token saved to {}", path.display());
            }
            #[cfg(feature = "mcp")]
            Command::Mcp(action) => {
                Outlook
                    .exec(action)
                    .await
                    .map_err(|e| Error::Api(e.to_string()))?;
            }
        }
        Ok(())
    }
}
