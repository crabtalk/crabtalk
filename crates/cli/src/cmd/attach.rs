//! Attach to an agent via the interactive chat REPL.

use crate::repl::{ChatRepl, runner::Runner};
use anyhow::Result;
use clap::Args;
use compact_str::CompactString;
use std::path::Path;
use wcore::paths::CONFIG_DIR;

/// Attach to an agent and start an interactive chat REPL.
#[derive(Args, Debug)]
pub struct Attach {
    /// Connect via TCP instead of Unix domain socket.
    /// Reads the port from ~/.openwalrus/walrus.tcp.
    #[arg(long, default_missing_value = "true", num_args = 0)]
    pub tcp: bool,
}

impl Attach {
    /// Enter the interactive REPL with the given runner and agent.
    pub async fn run(self, runner: Runner, agent: CompactString) -> Result<()> {
        let mut repl = ChatRepl::new(runner, agent)?;
        repl.run().await
    }
}

/// Check if providers are configured; prompt and reload the daemon if empty.
pub async fn ensure_providers(socket_path: &Path) -> Result<()> {
    let config_path = CONFIG_DIR.join("walrus.toml");
    if !config_path.exists() {
        return Ok(());
    }

    let config = ::daemon::DaemonConfig::load(&config_path)?;
    if config.provider.is_empty() {
        crate::cmd::daemon::setup_provider(&config_path)?;
        // Tell the running daemon to pick up the new config.
        if let Ok(mut runner) = Runner::connect(socket_path).await {
            let _ = runner.reload().await;
        }
    }
    Ok(())
}
