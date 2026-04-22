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
    /// for stdin), and a system prompt from `--prompt` or `--prompt-file`.
    Create {
        /// Agent name.
        name: String,
        /// Path to `AgentConfig` JSON. Use `-` to read from stdin. If omitted,
        /// the daemon receives `{}` and fills defaults.
        #[arg(long)]
        config: Option<PathBuf>,
        /// System prompt as an inline string.
        #[arg(long, conflicts_with = "prompt_file")]
        prompt: Option<String>,
        /// Read system prompt from a file (or `-` for stdin).
        #[arg(long)]
        prompt_file: Option<PathBuf>,
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
        let mut runner = super::connect(tcp).await?;
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
                prompt,
                prompt_file,
            } => {
                let config_json = match config {
                    Some(path) => super::read_path_or_stdin(&path)?,
                    None => "{}".to_string(),
                };
                let prompt_text = match (prompt, prompt_file) {
                    (Some(p), _) => p,
                    (None, Some(path)) => super::read_path_or_stdin(&path)?,
                    (None, None) => String::new(),
                };
                let info = runner.create_agent(name, config_json, prompt_text).await?;
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
