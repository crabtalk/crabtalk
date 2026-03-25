//! Interactive chat REPL with streaming output and persistent history.

use crate::repl::{
    command::{SlashResult, handle_slash},
    input::{History, InputResult},
    render::{MarkdownRenderer, styled_prompt, welcome_banner},
    runner::{ConnectionInfo, OutputChunk, Runner, send_reply},
};
use anyhow::Result;
use futures_core::Stream;
use futures_util::StreamExt;
use std::{path::PathBuf, pin::pin};
use wcore::protocol::message::AskQuestion;

mod ask;
pub mod command;
mod input;
pub mod render;
pub mod runner;

/// Interactive chat REPL.
pub struct ChatRepl {
    runner: Runner,
    agent: String,
    history: History,
    history_path: Option<PathBuf>,
}

impl ChatRepl {
    /// Create a new REPL with the given runner and agent name.
    pub fn new(runner: Runner, agent: String) -> Result<Self> {
        let history_path = history_file_path();
        let mut history = History::new();
        if let Some(ref path) = history_path {
            history.load(path);
        }
        Ok(Self {
            runner,
            agent,
            history,
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
        let mut new_chat = false;
        loop {
            let line = match input::read_line(&prompt, &mut self.history) {
                InputResult::Line(line) => line,
                InputResult::Interrupt => continue,
                InputResult::Eof => break,
                InputResult::ClearScreen => {
                    let _ = crossterm::execute!(
                        std::io::stdout(),
                        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
                        crossterm::cursor::MoveTo(0, 0),
                    );
                    println!("{}", render::welcome_banner(None));
                    println!();
                    continue;
                }
            };
            if line.is_empty() {
                continue;
            }
            self.history.push(&line);
            let content = match handle_slash(&line).await? {
                SlashResult::Handled => continue,
                SlashResult::NotSlash => line,
                SlashResult::Forward(cmd) => {
                    // Show the slash command dimmed.
                    println!("{}", console::style(&cmd).dim());
                    cmd
                }
                SlashResult::Exit => break,
                SlashResult::Clear => {
                    new_chat = true;
                    // Clear the screen and move cursor to top.
                    let _ = crossterm::execute!(
                        std::io::stdout(),
                        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
                        crossterm::cursor::MoveTo(0, 0),
                    );
                    println!("{}", render::welcome_banner(None));
                    println!();
                    continue;
                }
            };
            // Echo user input with gray background.
            println!("\x1b[48;5;236m {} \x1b[0m", content);
            println!();
            let conn_info = self.runner.conn_info().clone();
            let stream = self.runner.stream(&self.agent, &content, None, new_chat);
            stream_to_terminal(stream, &conn_info).await?;
            new_chat = false;
            println!();
        }

        println!();
        self.save_history();
        Ok(())
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

    /// Save history to disk.
    fn save_history(&self) {
        if let Some(ref path) = self.history_path {
            self.history.save(path);
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
                        match ask_user_interactive(&questions).await {
                            Ok(reply) => {
                                if let Err(e) = send_reply(conn_info, session, reply).await {
                                    eprintln!("failed to send reply: {e}");
                                }
                                // Reset renderer — the ask TUI took over the terminal,
                                // so cursor tracking in the old renderer is invalid.
                                println!();
                                renderer = MarkdownRenderer::new();
                                // Skip the ask_user tool result echo.
                                skip_tool_result += 1;
                            }
                            Err(_) => {
                                // User cancelled (Ctrl+C / Esc) — abort this
                                // response but keep the session alive.
                                break;
                            }
                        }
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
