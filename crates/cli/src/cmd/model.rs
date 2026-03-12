//! Model management command.

use crate::repl::runner::Runner;
use anyhow::Result;
use clap::{Args, Subcommand};
use futures_util::StreamExt;
use std::io::Write;
use wcore::protocol::message::DownloadEvent;

/// Manage local models.
#[derive(Args, Debug)]
pub struct Model {
    /// Model subcommand.
    #[command(subcommand)]
    pub command: ModelCommand,
}

/// Model subcommands.
#[derive(Subcommand, Debug)]
pub enum ModelCommand {
    /// Download a model from HuggingFace.
    Download(ModelId),
}

/// Model identifier argument.
#[derive(Args, Debug)]
pub struct ModelId {
    /// HuggingFace model ID (e.g. `microsoft/Phi-3.5-mini-instruct`).
    pub model: String,
}

impl Model {
    /// Run the model command.
    pub async fn run(self, runner: &mut Runner) -> Result<()> {
        let ModelCommand::Download(m) = self.command;
        let stream = runner.download_stream(&m.model);
        futures_util::pin_mut!(stream);
        let mut downloaded: u64 = 0;
        while let Some(result) = stream.next().await {
            match result? {
                DownloadEvent::Created { label, .. } => {
                    println!("Downloading {label}...");
                }
                DownloadEvent::Step { message, .. } => {
                    println!("  {message}");
                }
                DownloadEvent::Progress {
                    bytes, total_bytes, ..
                } => {
                    downloaded += bytes;
                    let pct = if total_bytes > 0 {
                        downloaded * 100 / total_bytes
                    } else {
                        0
                    };
                    eprint!(
                        "\r  {}% ({} / {})",
                        pct,
                        format_bytes(downloaded),
                        format_bytes(total_bytes),
                    );
                    std::io::stderr().flush().ok();
                }
                DownloadEvent::Completed { .. } => {
                    eprintln!();
                    println!("Download complete: {}", m.model);
                }
                DownloadEvent::Failed { error, .. } => {
                    eprintln!();
                    anyhow::bail!("download failed: {error}");
                }
            }
        }
        Ok(())
    }
}

/// Format a byte count as a human-readable string.
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}
