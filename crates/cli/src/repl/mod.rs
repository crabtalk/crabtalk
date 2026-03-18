//! Interactive chat REPL with streaming output and persistent history.

use crate::repl::{
    command::{ReplHelper, SlashResult, handle_slash},
    render::{MarkdownRenderer, styled_prompt, welcome_banner},
    runner::{OutputChunk, Runner},
};
use anyhow::Result;
use futures_core::Stream;
use futures_util::StreamExt;
use rustyline::{Editor, error::ReadlineError, history::DefaultHistory};
use std::{path::PathBuf, pin::pin};

pub mod command;
pub mod render;
pub mod runner;

/// Interactive chat REPL.
pub struct ChatRepl {
    runner: Runner,
    agent: String,
    editor: Editor<ReplHelper, DefaultHistory>,
    history_path: Option<PathBuf>,
}

impl ChatRepl {
    /// Create a new REPL with the given runner and agent name.
    pub fn new(runner: Runner, agent: String) -> Result<Self> {
        let mut editor = Editor::new()?;
        editor.set_helper(Some(ReplHelper));
        let history_path = history_file_path();
        if let Some(ref path) = history_path {
            let _ = editor.load_history(path);
        }
        Ok(Self {
            runner,
            agent,
            editor,
            history_path,
        })
    }

    /// Run the interactive REPL loop.
    pub async fn run(&mut self) -> Result<()> {
        // Fetch model name for banner (best-effort).
        let model = self.fetch_model_name().await;
        println!("{}", welcome_banner(model.as_deref()));
        println!();

        let prompt = styled_prompt(&self.agent);
        loop {
            match self.editor.readline(&prompt) {
                Ok(line) => {
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        continue;
                    }
                    let _ = self.editor.add_history_entry(&line);
                    let content = match handle_slash(&mut self.agent, &line).await? {
                        SlashResult::Handled => continue,
                        SlashResult::NotSlash => line,
                        SlashResult::Forward(cmd) => {
                            // Show the slash command dimmed.
                            println!("{}", console::style(&cmd).dim());
                            cmd
                        }
                    };
                    println!();
                    let stream = self.runner.stream(&self.agent, &content);
                    stream_to_terminal(stream).await?;
                    println!();
                }
                Err(ReadlineError::Interrupted) => continue,
                Err(ReadlineError::Eof) => break,
                Err(e) => return Err(e.into()),
            }
        }

        self.save_history();
        Ok(())
    }

    /// Try to extract the model name from daemon config.
    async fn fetch_model_name(&mut self) -> Option<String> {
        let config_json = self.runner.get_config().await.ok()?;
        let val: serde_json::Value = serde_json::from_str(&config_json).ok()?;
        val.get("system")?
            .get("walrus")?
            .get("model")?
            .as_str()
            .map(|s| s.to_string())
    }

    /// Save readline history to disk.
    fn save_history(&mut self) {
        if let Some(ref path) = self.history_path {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = self.editor.save_history(path);
        }
    }
}

/// Resolve the history file path at `~/.openwalrus/history`.
fn history_file_path() -> Option<PathBuf> {
    Some(wcore::paths::CONFIG_DIR.join("history"))
}

/// Consume a stream of output chunks and render them via `MarkdownRenderer`.
async fn stream_to_terminal(stream: impl Stream<Item = Result<OutputChunk>>) -> Result<()> {
    let mut stream = pin!(stream);
    let mut renderer = MarkdownRenderer::new();

    loop {
        tokio::select! {
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(OutputChunk::Text(text))) => {
                        renderer.push_text(&text);
                    }
                    Some(Ok(OutputChunk::Thinking(text))) => {
                        renderer.push_thinking(&text);
                    }
                    Some(Ok(OutputChunk::ToolStart(names))) => {
                        renderer.push_tool_start(&names);
                    }
                    Some(Ok(OutputChunk::ToolDone)) => {
                        renderer.push_tool_done();
                    }
                    Some(Err(e)) => {
                        renderer.finish();
                        eprintln!("\nError: {e}");
                        break;
                    }
                    None => break,
                }
            }
            _ = tokio::signal::ctrl_c() => {
                renderer.finish();
                println!();
                break;
            }
        }
    }

    renderer.finish();
    Ok(())
}
