//! Gateway serve command — connect to daemon and spawn platform bots.

use crate::{client::DaemonClient, config::GatewayConfig};
use std::{path::Path, sync::Arc};

/// Run the gateway service.
///
/// Parses the JSON config, builds a `DaemonClient`, resolves the default
/// agent name from the agents directory, spawns platform bots, and blocks
/// until SIGINT.
pub async fn run(daemon_socket: &str, config_json: &str) -> anyhow::Result<()> {
    let config: GatewayConfig = serde_json::from_str(config_json)?;
    let client = Arc::new(DaemonClient::new(Path::new(daemon_socket)));

    // Resolve default agent from agents dir (first .md file stem).
    let agents_dir = wcore::paths::CONFIG_DIR.join(wcore::paths::AGENTS_DIR);
    let default_agent = resolve_default_agent(&agents_dir);
    tracing::info!(agent = %default_agent, "gateway starting");

    crate::spawn::spawn_gateways(&config, default_agent, client).await;

    // Block until ctrl-c.
    tokio::signal::ctrl_c().await?;
    tracing::info!("gateway shutting down");
    Ok(())
}

/// Read the agents directory and return the first agent name found,
/// falling back to "assistant".
fn resolve_default_agent(agents_dir: &Path) -> compact_str::CompactString {
    let Ok(entries) = std::fs::read_dir(agents_dir) else {
        return compact_str::CompactString::from("assistant");
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "md")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            return compact_str::CompactString::from(stem);
        }
    }
    compact_str::CompactString::from("assistant")
}
