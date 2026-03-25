//! MCP server management commands.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use daemon::hook::mcp::auth::FileCredentialStore;
use rmcp::transport::{AuthorizationManager, AuthorizationSession};
use std::sync::Arc;

/// Manage MCP servers.
#[derive(Args, Debug)]
pub struct Mcp {
    /// Subcommand.
    #[command(subcommand)]
    pub command: McpCommand,
}

/// MCP subcommands.
#[derive(Subcommand, Debug)]
pub enum McpCommand {
    /// Authenticate with an MCP server via OAuth.
    Login(McpLogin),
    /// Remove stored OAuth tokens for an MCP server.
    Logout(McpLogout),
}

/// Login arguments.
#[derive(Args, Debug)]
pub struct McpLogin {
    /// MCP server name (as defined in manifest).
    pub name: String,
}

/// Logout arguments.
#[derive(Args, Debug)]
pub struct McpLogout {
    /// MCP server name.
    pub name: String,
}

/// Callback port for the local OAuth redirect server.
const CALLBACK_PORT: u16 = 19836;

impl Mcp {
    /// Run the MCP subcommand.
    pub async fn run(self) -> Result<()> {
        match self.command {
            McpCommand::Login(cmd) => login(&cmd.name).await,
            McpCommand::Logout(cmd) => logout(&cmd.name),
        }
    }
}

/// Look up an MCP server URL by name from resolved manifests.
fn resolve_mcp_url(name: &str) -> Result<String> {
    let config_dir = &*wcore::paths::CONFIG_DIR;
    let (manifest, _) = wcore::resolve_manifests(config_dir);
    let mcp = manifest
        .mcps
        .get(name)
        .with_context(|| format!("MCP server '{name}' not found in manifests"))?;
    mcp.url
        .clone()
        .with_context(|| format!("MCP server '{name}' uses stdio transport — OAuth not applicable"))
}

/// Run the OAuth login flow for an MCP server.
async fn login(name: &str) -> Result<()> {
    let url = resolve_mcp_url(name)?;
    let redirect_uri = format!("http://localhost:{CALLBACK_PORT}/callback");

    println!("Discovering OAuth metadata for {url} …");
    let mut manager = AuthorizationManager::new(&url)
        .await
        .context("failed to discover OAuth metadata — server may not support auth")?;
    manager.set_credential_store(FileCredentialStore::for_server(name));

    println!("Registering client …");
    let session = AuthorizationSession::new(manager, &[], &redirect_uri, Some("crabtalk"), None)
        .await
        .context("OAuth registration failed")?;

    let auth_url = session.get_authorization_url().to_owned();
    println!("Opening browser for authorization …");
    if let Err(e) = open::that(&auth_url) {
        println!("Failed to open browser: {e}");
        println!("Open this URL manually:\n  {auth_url}");
    }

    // Shared state for the callback handler.
    let session = Arc::new(session);
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let state = CallbackState {
        session: Arc::clone(&session),
        tx: Arc::new(tokio::sync::Mutex::new(Some(tx))),
    };

    let app = axum::Router::new()
        .route("/callback", axum::routing::get(handle_callback))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{CALLBACK_PORT}"))
        .await
        .with_context(|| format!("failed to bind callback server on port {CALLBACK_PORT}"))?;
    println!("Waiting for callback on {redirect_uri} …");

    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    // Wait for the callback or timeout.
    let result = tokio::time::timeout(std::time::Duration::from_secs(120), rx).await;

    server.abort();

    match result {
        Ok(Ok(Ok(()))) => {
            println!("Token saved to ~/.crabtalk/tokens/{name}.json");
            println!("Run `crabtalk daemon reload` to connect with the new credentials.");
            Ok(())
        }
        Ok(Ok(Err(msg))) => anyhow::bail!("authorization failed: {msg}"),
        Ok(Err(_)) => anyhow::bail!("callback channel dropped unexpectedly"),
        Err(_) => anyhow::bail!("authorization timed out (2 minutes)"),
    }
}

/// Remove stored OAuth tokens for an MCP server.
fn logout(name: &str) -> Result<()> {
    let path = wcore::paths::TOKENS_DIR.join(format!("{name}.json"));
    match std::fs::remove_file(&path) {
        Ok(()) => println!("Removed token for '{name}'."),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("No token found for '{name}'.");
        }
        Err(e) => anyhow::bail!("failed to remove {}: {e}", path.display()),
    }
    Ok(())
}

/// Query parameters from the OAuth callback redirect.
#[derive(serde::Deserialize)]
struct CallbackParams {
    code: String,
    state: String,
}

/// Shared state passed to the axum callback handler.
#[derive(Clone)]
struct CallbackState {
    session: Arc<AuthorizationSession>,
    tx: Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<Result<(), String>>>>>,
}

/// Axum handler for the OAuth callback.
async fn handle_callback(
    axum::extract::State(state): axum::extract::State<CallbackState>,
    axum::extract::Query(params): axum::extract::Query<CallbackParams>,
) -> axum::response::Html<&'static str> {
    let result = state
        .session
        .handle_callback(&params.code, &params.state)
        .await;
    match result {
        Ok(_) => {
            if let Some(tx) = state.tx.lock().await.take() {
                let _ = tx.send(Ok(()));
            }
            axum::response::Html("Authorization successful — you can close this tab.")
        }
        Err(ref e) => {
            let msg = format!("{e}");
            if let Some(tx) = state.tx.lock().await.take() {
                let _ = tx.send(Err(msg));
            }
            axum::response::Html("Authorization failed. Check the terminal for details.")
        }
    }
}
