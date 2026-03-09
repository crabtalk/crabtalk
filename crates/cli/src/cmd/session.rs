//! Session management command.

use crate::repl::runner::Runner;
use anyhow::Result;
use clap::{Args, Subcommand};

/// Manage active sessions.
#[derive(Args, Debug)]
pub struct Session {
    /// Session subcommand.
    #[command(subcommand)]
    pub command: SessionCommand,
}

/// Session subcommands.
#[derive(Subcommand, Debug)]
pub enum SessionCommand {
    /// List active sessions.
    List,
    /// Kill (close) a session.
    Kill {
        /// Session ID to close.
        id: u64,
    },
}

impl Session {
    /// Run the session command.
    pub async fn run(self, runner: &mut Runner) -> Result<()> {
        match self.command {
            SessionCommand::List => {
                let sessions = runner.list_sessions().await?;
                if sessions.is_empty() {
                    println!("No active sessions.");
                    return Ok(());
                }
                println!(
                    "{:<6} {:<16} {:<16} {:<8} {:<10}",
                    "ID", "AGENT", "CREATED BY", "MSGS", "ALIVE"
                );
                for s in sessions {
                    let alive = format_duration(s.alive_secs);
                    println!(
                        "{:<6} {:<16} {:<16} {:<8} {:<10}",
                        s.id, s.agent, s.created_by, s.message_count, alive
                    );
                }
            }
            SessionCommand::Kill { id } => {
                if runner.kill_session(id).await? {
                    println!("Session {id} closed.");
                } else {
                    anyhow::bail!("session {id} not found");
                }
            }
        }
        Ok(())
    }
}

/// Format seconds into a human-readable duration.
fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}
