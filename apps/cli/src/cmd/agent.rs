//! `crabtalk agent` — non-interactive agent CRUD.

use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::PathBuf;
use wcore::protocol::api::Client;

/// Manage agents.
#[derive(Args, Debug)]
pub struct Agent {
    #[command(subcommand)]
    pub command: AgentCmd,
}

#[derive(Subcommand, Debug)]
pub enum AgentCmd {
    /// List registered agents.
    List,
    /// Create an agent. Reads `AgentConfig` JSON from `--config` (file or `-`
    /// for stdin), and its description from `--description` or
    /// `--description-file`. The description is the system message.
    Create {
        /// Agent name.
        name: String,
        /// Path to `AgentConfig` JSON. Use `-` to read from stdin. If omitted,
        /// the daemon receives `{}` and fills defaults.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Description as an inline string.
        #[arg(long, conflicts_with = "description_file")]
        description: Option<String>,
        /// Read the description from a file (or `-` for stdin).
        #[arg(long)]
        description_file: Option<PathBuf>,
    },
    /// Delete an agent by name.
    Delete {
        /// Agent name.
        name: String,
    },
    /// Rename an agent in place. The stored ULID stays stable.
    Rename {
        /// Existing agent name.
        old_name: String,
        /// New agent name.
        new_name: String,
    },
}

impl Agent {
    pub async fn run(self, tcp: bool) -> Result<()> {
        let (mut runner, _) = super::connect(tcp).await?;
        match self.command {
            AgentCmd::List => {
                let agents = runner.list_agents().await?;
                if agents.is_empty() {
                    return Ok(());
                }
                let name_w = agents.iter().map(|a| a.name.len()).max().unwrap_or(0);
                for a in agents {
                    let model = if a.model.is_empty() { "-" } else { &a.model };
                    println!("{:<name_w$}  {}", a.name, model);
                }
            }
            AgentCmd::Create {
                name,
                config,
                description,
                description_file,
            } => {
                let config_json = match config {
                    Some(path) => super::read_path_or_stdin(&path)?,
                    None => "{}".to_string(),
                };
                let described = match (description, description_file) {
                    (Some(text), _) => Some(text),
                    (None, Some(path)) => Some(super::read_path_or_stdin(&path)?),
                    (None, None) => None,
                };
                // Merged into the config rather than sent beside it: the
                // description *is* the config's description, and a flag that
                // silently lost to a `--config` field would be worse than no
                // flag at all.
                let config_json = match described {
                    Some(text) => {
                        let mut value: serde_json::Value = serde_json::from_str(&config_json)?;
                        value["description"] = serde_json::Value::String(text);
                        serde_json::to_string(&value)?
                    }
                    None => config_json,
                };
                let info = runner.create_agent(name, config_json).await?;
                println!("saved '{}'", info.name);
            }
            AgentCmd::Delete { name } => {
                runner.delete_agent(name).await?;
            }
            AgentCmd::Rename { old_name, new_name } => {
                let info = runner.rename_agent(old_name, new_name).await?;
                println!("saved '{}'", info.name);
            }
        }
        Ok(())
    }
}
