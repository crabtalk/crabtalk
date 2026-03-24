//! Interactive chat REPL with streaming output and persistent history.

use crate::repl::{
    command::{ReplHelper, SlashResult, handle_slash},
    render::{MarkdownRenderer, styled_prompt, welcome_banner},
    runner::{ConnectionInfo, OutputChunk, Runner, send_reply},
};
use anyhow::Result;
use futures_core::Stream;
use futures_util::StreamExt;
use rustyline::{Editor, config::CompletionType, error::ReadlineError, history::DefaultHistory};
use std::{path::PathBuf, pin::pin};
use wcore::protocol::message::AskQuestion;

mod ask;
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
        let config = rustyline::Config::builder()
            .completion_type(CompletionType::List)
            .build();
        let mut editor = Editor::with_config(config)?;
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
        let cont_prompt = "  .. ";
        loop {
            let line = match self.read_input(&prompt, cont_prompt) {
                Ok(line) => line,
                Err(ReadlineError::Interrupted) => continue,
                Err(ReadlineError::Eof) => break,
                Err(e) => return Err(e.into()),
            };
            if line.is_empty() {
                continue;
            }
            let _ = self.editor.add_history_entry(&line);
            let content = match handle_slash(&line).await? {
                SlashResult::Handled => continue,
                SlashResult::NotSlash => line,
                SlashResult::Forward(cmd) => {
                    // Show the slash command dimmed.
                    println!("{}", console::style(&cmd).dim());
                    cmd
                }
            };
            println!();
            let conn_info = self.runner.conn_info().clone();
            let stream = self.runner.stream(&self.agent, &content, None);
            stream_to_terminal(stream, &conn_info).await?;
            println!();
        }

        self.save_history();
        Ok(())
    }

    /// Read a potentially multi-line input.
    ///
    /// Lines ending with `\` are continued on the next line. The backslash
    /// is stripped and a newline is inserted in its place.
    fn read_input(&mut self, prompt: &str, cont_prompt: &str) -> rustyline::Result<String> {
        let mut buf = String::new();
        let mut first = true;
        loop {
            let p = if first { prompt } else { cont_prompt };
            first = false;
            let line = self.editor.readline(p)?;
            let trimmed = line.trim_end();
            if let Some(prefix) = trimmed.strip_suffix('\\') {
                buf.push_str(prefix);
                buf.push('\n');
            } else {
                buf.push_str(trimmed);
                break;
            }
        }
        let result = buf.trim().to_string();
        Ok(result)
    }

    /// Try to extract the model name from daemon config.
    async fn fetch_model_name(&mut self) -> Option<String> {
        let config_json = self.runner.get_config().await.ok()?;
        let val: serde_json::Value = serde_json::from_str(&config_json).ok()?;
        val.get("system")?
            .get("crab")?
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

/// Resolve the history file path at `~/.crabtalk/history`.
fn history_file_path() -> Option<PathBuf> {
    Some(wcore::paths::CONFIG_DIR.join("history"))
}

/// Consume a stream of output chunks and render them via `MarkdownRenderer`.
pub(crate) async fn stream_to_terminal(
    stream: impl Stream<Item = Result<OutputChunk>>,
    conn_info: &ConnectionInfo,
) -> Result<()> {
    let mut stream = pin!(stream);
    let mut renderer = MarkdownRenderer::new();
    renderer.start_waiting();
    // After an AskUser interaction, skip the echoed ToolResult + ToolDone
    // for the ask_user tool — the user already saw their own answer.
    let mut skip_tool_result: u32 = 0;

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
                    Some(Ok(OutputChunk::ToolStart(calls))) => {
                        renderer.push_tool_start(&calls);
                    }
                    Some(Ok(OutputChunk::ToolResult(_id, output))) => {
                        if skip_tool_result > 0 {
                            skip_tool_result -= 1;
                        } else {
                            renderer.push_tool_result(&output);
                        }
                    }
                    Some(Ok(OutputChunk::ToolDone(success))) => {
                        renderer.push_tool_done(success);
                    }
                    Some(Ok(OutputChunk::AskUser { questions, session })) => {
                        renderer.finish();
                        println!();
                        let reply = ask_user_interactive(&questions).await?;
                        if let Err(e) = send_reply(conn_info, session, reply).await {
                            eprintln!("failed to send reply: {e}");
                        }
                        // Reset renderer — the ask TUI took over the terminal,
                        // so cursor tracking in the old renderer is invalid.
                        renderer = MarkdownRenderer::new();
                        // Skip the ask_user tool result echo.
                        skip_tool_result += 1;
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

/// Present structured questions via inline ratatui TUI.
///
/// Returns a JSON string mapping question text to selected label(s).
async fn ask_user_interactive(questions: &[AskQuestion]) -> Result<String> {
    let questions = questions.to_vec();
    tokio::task::spawn_blocking(move || {
        let answers = ask::run_ask_inline(&questions)?;
        Ok(serde_json::to_string(&answers)?)
    })
    .await?
}
