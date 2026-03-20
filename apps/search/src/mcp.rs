use crate::aggregator::Aggregator;
use crate::browser::fetch;
use crate::config::Config;
use reqwest::Client;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

pub struct SearchServer {
    tool_router: ToolRouter<Self>,
    aggregator: Aggregator,
    client: Client,
}

impl Default for SearchServer {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchServer {
    pub fn new() -> Self {
        let config = Config::discover();
        let aggregator = Aggregator::new(config).expect("failed to create aggregator");
        let client = fetch::default_client().expect("failed to create HTTP client");
        Self {
            tool_router: Self::tool_router(),
            aggregator,
            client,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchParams {
    /// The search query string.
    pub query: String,
    /// Page number (0-indexed). Defaults to 0.
    pub page: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FetchParams {
    /// The URL to fetch and extract text from.
    pub url: String,
}

#[tool_router]
impl SearchServer {
    #[tool(description = "Search the web using multiple engines and return aggregated results")]
    async fn search(&self, Parameters(params): Parameters<SearchParams>) -> String {
        let page = params.page.unwrap_or(0);
        match self.aggregator.search(&params.query, page).await {
            Ok(results) => serde_json::to_string(&results).unwrap_or_default(),
            Err(e) => format!("search error: {e}"),
        }
    }

    #[tool(description = "Fetch a web page and extract its text content")]
    async fn fetch(&self, Parameters(params): Parameters<FetchParams>) -> String {
        match fetch::fetch_url(&params.url, &self.client).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_default(),
            Err(e) => format!("fetch error: {e}"),
        }
    }
}

#[tool_handler]
impl ServerHandler for SearchServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Meta search engine with aggregated results from multiple backends".into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
