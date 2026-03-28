use crate::{calendar, client::OutlookClient, config::Config, mail};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

pub struct OutlookServer {
    tool_router: ToolRouter<Self>,
    client: OutlookClient,
}

impl Default for OutlookServer {
    fn default() -> Self {
        Self::new()
    }
}

impl OutlookServer {
    pub fn new() -> Self {
        let config = Config::load().expect("failed to load outlook config");
        let client = OutlookClient::new(config).expect("failed to create outlook client");
        Self {
            tool_router: Self::tool_router(),
            client,
        }
    }
}

// ── Mail params ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListMailParams {
    /// Number of messages to return (default: 25, max: 50).
    pub count: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadMailParams {
    /// The message ID.
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchMailParams {
    /// Search query (searches subject, body, sender, etc.).
    pub query: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendMailParams {
    /// Recipient email address.
    pub to: String,
    /// Email subject.
    pub subject: String,
    /// Email body (plain text).
    pub body: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReplyMailParams {
    /// The message ID to reply to.
    pub id: String,
    /// Reply body (plain text).
    pub body: String,
}

// ── Calendar params ─────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListEventsParams {
    /// Start of the time range (ISO 8601, e.g. "2025-01-01T00:00:00").
    pub start: String,
    /// End of the time range (ISO 8601, e.g. "2025-01-31T23:59:59").
    pub end: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateEventParams {
    /// Event subject/title.
    pub subject: String,
    /// Start time (ISO 8601, e.g. "2025-01-15T10:00:00").
    pub start: String,
    /// End time (ISO 8601, e.g. "2025-01-15T11:00:00").
    pub end: String,
    /// IANA time zone (e.g. "America/New_York"). Defaults to "UTC".
    pub time_zone: Option<String>,
    /// Location name.
    pub location: Option<String>,
    /// Event body/description (plain text).
    pub body: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateEventParams {
    /// The event ID to update.
    pub id: String,
    /// New subject (optional).
    pub subject: Option<String>,
    /// New start time (optional, ISO 8601).
    pub start: Option<String>,
    /// New end time (optional, ISO 8601).
    pub end: Option<String>,
    /// IANA time zone for start/end (optional).
    pub time_zone: Option<String>,
    /// New location (optional).
    pub location: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteEventParams {
    /// The event ID to delete.
    pub id: String,
}

// ── Tools ───────────────────────────────────────────────────────────

#[tool_router]
impl OutlookServer {
    #[tool(description = "List recent emails from the inbox, ordered by date")]
    async fn list_mail(&self, Parameters(params): Parameters<ListMailParams>) -> String {
        let count = params.count.unwrap_or(25).min(50);
        match mail::list_mail(&self.client, count).await {
            Ok(result) => result,
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(description = "Read the full content of an email by its ID")]
    async fn read_mail(&self, Parameters(params): Parameters<ReadMailParams>) -> String {
        match mail::read_mail(&self.client, &params.id).await {
            Ok(result) => result,
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(description = "Search emails by query (searches subject, body, sender)")]
    async fn search_mail(&self, Parameters(params): Parameters<SearchMailParams>) -> String {
        match mail::search_mail(&self.client, &params.query).await {
            Ok(result) => result,
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(description = "Send a new email")]
    async fn send_mail(&self, Parameters(params): Parameters<SendMailParams>) -> String {
        match mail::send_mail(&self.client, &params.to, &params.subject, &params.body).await {
            Ok(result) => result,
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(description = "Reply to an email by its ID")]
    async fn reply_mail(&self, Parameters(params): Parameters<ReplyMailParams>) -> String {
        match mail::reply_mail(&self.client, &params.id, &params.body).await {
            Ok(result) => result,
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(description = "List calendar events within a time range")]
    async fn list_events(&self, Parameters(params): Parameters<ListEventsParams>) -> String {
        match calendar::list_events(&self.client, &params.start, &params.end).await {
            Ok(result) => result,
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(description = "Create a new calendar event")]
    async fn create_event(&self, Parameters(params): Parameters<CreateEventParams>) -> String {
        let tz = params.time_zone.as_deref().unwrap_or("UTC");
        match calendar::create_event(
            &self.client,
            &params.subject,
            &params.start,
            &params.end,
            tz,
            params.location.as_deref(),
            params.body.as_deref(),
        )
        .await
        {
            Ok(result) => result,
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(description = "Update an existing calendar event")]
    async fn update_event(&self, Parameters(params): Parameters<UpdateEventParams>) -> String {
        match calendar::update_event(
            &self.client,
            &params.id,
            params.subject.as_deref(),
            params.start.as_deref(),
            params.end.as_deref(),
            params.time_zone.as_deref(),
            params.location.as_deref(),
        )
        .await
        {
            Ok(result) => result,
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(description = "Delete a calendar event by its ID")]
    async fn delete_event(&self, Parameters(params): Parameters<DeleteEventParams>) -> String {
        match calendar::delete_event(&self.client, &params.id).await {
            Ok(result) => result,
            Err(e) => format!("error: {e}"),
        }
    }
}

#[tool_handler]
impl ServerHandler for OutlookServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Outlook MCP server for email and calendar automation via Microsoft Graph API",
        )
    }
}
