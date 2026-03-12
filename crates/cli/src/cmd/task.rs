//! Task management command.

use crate::repl::runner::Runner;
use anyhow::Result;
use clap::{Args, Subcommand};

/// Manage tasks in the task registry.
#[derive(Args, Debug)]
pub struct Task {
    /// Task subcommand.
    #[command(subcommand)]
    pub command: TaskCommand,
}

/// Task subcommands.
#[derive(Subcommand, Debug)]
pub enum TaskCommand {
    /// List tasks.
    List,
    /// Kill (cancel) a task.
    Kill {
        /// Task ID to cancel.
        id: u64,
    },
    /// Approve a blocked task's inbox question.
    Approve {
        /// Task ID to approve.
        id: u64,
        /// Response to send to the blocked task.
        response: String,
    },
}

impl Task {
    /// Run the task command.
    pub async fn run(self, runner: &mut Runner) -> Result<()> {
        match self.command {
            TaskCommand::List => {
                let tasks = runner.list_tasks().await?;
                if tasks.is_empty() {
                    println!("No active tasks.");
                    return Ok(());
                }
                println!(
                    "{:<6} {:<16} {:<12} {:<10} {:<10}",
                    "ID", "AGENT", "STATUS", "ALIVE", "TOKENS"
                );
                for t in tasks {
                    let alive = format_duration(t.alive_secs);
                    let tokens = t.prompt_tokens + t.completion_tokens;
                    println!(
                        "{:<6} {:<16} {:<12} {:<10} {:<10}",
                        t.id, t.agent, t.status, alive, tokens
                    );
                    if let Some(q) = &t.blocked_on {
                        println!("       blocked: {q}");
                    }
                }
            }
            TaskCommand::Kill { id } => {
                if runner.kill_task(id).await? {
                    println!("Task {id} killed.");
                } else {
                    anyhow::bail!("task {id} not found or already completed");
                }
            }
            TaskCommand::Approve { id, response } => {
                if runner.approve_task(id, response).await? {
                    println!("Task {id} approved.");
                } else {
                    anyhow::bail!("task {id} not found or not blocked");
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
