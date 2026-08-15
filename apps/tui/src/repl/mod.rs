//! Full-screen interactive chat REPL with concurrent input and streaming.

use crate::repl::{
    ask::{AskAction, AskState},
    chat::{ChatEntry, TaskProgress, ToolStatus},
    command::{SlashResult, handle_slash},
    input::{History, InputAction, InputState},
    render::MarkdownRenderer,
};
use anyhow::Result;
use client::{ConnectionInfo, OutputChunk, Transport};
use crossterm::event::{Event, EventStream, KeyCode, KeyModifiers};
use futures_util::StreamExt;
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
    time::Duration,
};
use tokio::sync::mpsc;
use wcore::protocol::api::Client as _;
use wcore::protocol::message::StreamMsg;

mod ask;
pub mod chat;
pub mod command;
pub mod delegate;
pub mod input;
mod instructions;
pub mod render;
pub mod tools;

/// Interactive chat REPL.
pub struct ChatRepl {
    runner: Transport,
    conn_info: ConnectionInfo,
    agent: String,
    history_path: Option<PathBuf>,
    history: History,
}

impl ChatRepl {
    /// Create a new REPL with the given transport, conn_info, and agent name.
    pub fn new(runner: Transport, conn_info: ConnectionInfo, agent: String) -> Result<Self> {
        let history_path = history_file_path();
        let mut history = History::new();
        if let Some(ref path) = history_path {
            history.load(path);
        }
        Ok(Self {
            runner,
            conn_info,
            agent,
            history_path,
            history,
        })
    }

    /// Resume a specific conversation file in the interactive REPL.
    pub async fn resume(&mut self, _path: PathBuf) -> Result<()> {
        // Resume is no longer supported in the new protocol — conversations
        // are continuous per (agent, sender). Just start the normal REPL.
        self.run().await
    }

    /// Run the full-screen interactive REPL loop.
    pub async fn run(&mut self) -> Result<()> {
        // Sessions now live inside the daemon's Storage; the CLI can't
        // poke at them directly anymore. Start with an empty title —
        // the daemon publishes the generated title via stream events
        // after the first exchange.
        self.run_inner(String::new()).await
    }

    async fn run_inner(&mut self, chat_title: String) -> Result<()> {
        let model = self.fetch_model_name().await;
        let conn_info = self.conn_info.clone();
        let os_user = std::env::var("USER").unwrap_or_else(|_| "user".into());

        let skill_names: Vec<String> = self
            .runner
            .list_skills()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.name)
            .collect();
        let peers = delegate::peers_block(
            &self.runner.list_agents().await.unwrap_or_default(),
            &self.agent,
        );
        let history = std::mem::take(&mut self.history);
        let (progress_tx, progress_rx) = mpsc::unbounded_channel();
        let mut app = App {
            renderer: MarkdownRenderer::new(),
            input: InputState::new(history, skill_names),
            scroll: 0,
            message_queue: VecDeque::new(),
            agent: self.agent.clone(),
            chat_title,
            dirty: true,
            frame_count: 0,
            suppressed_results: HashSet::new(),
            streaming: false,
            conn_info,
            os_user,
            model_name: model,
            ask_state: None,
            ask_conversation_id: None,
            ask_call_id: None,
            forwards: Vec::new(),
            progress_tx,
            peers_pending: !peers.is_empty(),
            peers,
        };

        // Push welcome banner as first chat entry.
        app.renderer.buffer.push(ChatEntry::Text(vec![welcome_line(
            &app.agent,
            app.model_name.as_deref(),
        )]));

        let mut terminal = crate::tui::setup()?;
        let result = run_event_loop(&mut terminal, &mut app, progress_rx).await;

        crate::tui::teardown(&mut terminal)?;

        // Save history back.
        self.history = std::mem::take(&mut app.input.history);
        self.save_history();

        result
    }

    async fn fetch_model_name(&mut self) -> Option<String> {
        let stats = self.runner.get_stats().await.ok()?;
        if stats.active_model.is_empty() {
            None
        } else {
            Some(stats.active_model)
        }
    }

    fn save_history(&self) {
        if let Some(ref path) = self.history_path {
            self.history.save(path);
        }
    }
}

fn history_file_path() -> Option<PathBuf> {
    Some(wcore::paths::CONFIG_DIR.join("history"))
}

fn welcome_line(_agent: &str, model: Option<&str>) -> Line<'static> {
    let model_part = match model {
        Some(m) => format!(" ({m})"),
        None => String::new(),
    };
    Line::from(vec![
        Span::styled(
            format!("  Crabtalk{model_part}"),
            Style::new()
                .fg(Color::Indexed(173))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " — Ctrl+D to exit",
            Style::new()
                .fg(Color::Indexed(173))
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

// ── App state ────────────────────────────────────────────────────

struct App {
    renderer: MarkdownRenderer,
    input: InputState,
    scroll: usize,
    message_queue: VecDeque<String>,
    agent: String,
    chat_title: String,
    dirty: bool,
    frame_count: u64,
    /// Tool calls whose raw result the chat already showed in a better
    /// form (an ask modal, a fan-out task list). Keyed by call id rather
    /// than counted, so a step mixing these with an ordinary tool call
    /// suppresses the right one.
    suppressed_results: HashSet<String>,
    streaming: bool,
    conn_info: ConnectionInfo,
    os_user: String,
    model_name: Option<String>,
    /// Active ask-user modal (if any).
    ask_state: Option<AskState>,
    /// Conversation ID for the pending ask_user reply.
    ask_conversation_id: Option<u64>,
    /// Call ID for the pending ask_user reply.
    ask_call_id: Option<String>,
    /// Local OS tools dispatcher — answers forwarded tool calls from the
    /// daemon (bash, read, edit). Shared across stream turns so the
    /// "must read before edit" invariant persists.
    /// In-flight forwarded tool calls. Held so Ctrl+C can abort them —
    /// a `delegate` call outlives the keystroke otherwise, and its
    /// sub-agents keep running commands against the user's machine.
    forwards: Vec<tokio::task::JoinHandle<()>>,
    /// Fan-out progress from `delegate` executors back into the chat buffer.
    progress_tx: mpsc::UnboundedSender<delegate::Progress>,
    /// `<agents>` block naming this client's delegate targets. Sent once per
    /// conversation rather than every turn — it is a stable preamble, and
    /// repeating it would both waste tokens and move the prompt prefix.
    peers: String,
    /// Whether `peers` still needs to go out on the next message.
    peers_pending: bool,
}

impl App {
    fn track_forward(&mut self, handle: tokio::task::JoinHandle<()>) {
        self.forwards.retain(|h| !h.is_finished());
        self.forwards.push(handle);
    }

    fn abort_forwards(&mut self) {
        for handle in self.forwards.drain(..) {
            handle.abort();
        }
        self.renderer.buffer.cancel_running_tasks();
    }
}

// ── Event loop ───────────────────────────────────────────────────

async fn run_event_loop(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
    mut progress_rx: mpsc::UnboundedReceiver<delegate::Progress>,
) -> Result<()> {
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(33));
    let mut chunk_rx: Option<mpsc::UnboundedReceiver<Result<OutputChunk>>> = None;

    loop {
        // Draw when dirty.
        if app.dirty {
            let width = terminal.size()?.width as usize;
            app.renderer.set_width(width.saturating_sub(2));
            terminal.draw(|f| draw(f, app))?;
            app.dirty = false;
        }

        tokio::select! {
            // Branch 1: stream chunks from daemon.
            recv = async {
                if let Some(rx) = &mut chunk_rx {
                    rx.recv().await
                } else {
                    std::future::pending().await
                }
            } => {
                match recv {
                    Some(Ok(chunk)) => {
                        handle_chunk(chunk, app);
                        app.dirty = true;
                    }
                    Some(Err(e)) => {
                        app.renderer.finish();
                        app.renderer.buffer.push(ChatEntry::Text(vec![
                            Line::from(Span::styled(
                                format!("Error: {e}"),
                                Style::new().fg(Color::Red),
                            )),
                        ]));
                        chunk_rx = None;
                        app.streaming = false;
                        app.dirty = true;
                    }
                    None => {
                        // Stream ended.
                        app.renderer.finish();
                        chunk_rx = None;
                        app.streaming = false;
                        app.scroll = 0;
                        // Title is populated by the daemon through the
                        // stream event protocol — there's no longer a
                        // local file to poll.
                        // Send queued message if any.
                        if let Some(msg) = app.message_queue.pop_front() {
                            chunk_rx = Some(start_stream(app, &msg));
                        }
                        app.dirty = true;
                    }
                }
            }

            // Branch 2: terminal events.
            event = events.next() => {
                match event {
                    Some(Ok(Event::Key(key))) => {
                        // Ask modal intercepts all keys when active.
                        if app.ask_state.is_some() {
                            let action = app.ask_state.as_mut().unwrap().handle_key(key);
                            match action {
                                AskAction::Noop => {}
                                AskAction::Cancelled => {
                                    app.ask_state = None;
                                    app.ask_conversation_id = None;
                                    app.ask_call_id = None;
                                }
                                AskAction::Submitted(answers) => {
                                    let reply = serde_json::to_string(&answers).unwrap_or_default();
                                    if let (Some(conv_id), Some(call_id)) = (app.ask_conversation_id.take(), app.ask_call_id.take()) {
                                        let conn_info = app.conn_info.clone();
                                        app.suppressed_results.insert(call_id.clone());
                                        tokio::spawn(async move {
                                            let _ = conn_info.reply_to_tool(conv_id, call_id, reply, false).await;
                                        });
                                    }
                                    app.ask_state = None;
                                }
                            }
                            app.dirty = true;
                            continue;
                        }

                        // Scroll keys.
                        if key.code == KeyCode::PageUp {
                            let chat_lines = app.renderer.buffer.lines(app.frame_count).len();
                            app.scroll = app.scroll.saturating_add(10).min(chat_lines.saturating_sub(1));
                            app.dirty = true;
                            continue;
                        }
                        if key.code == KeyCode::PageDown {
                            app.scroll = app.scroll.saturating_sub(10);
                            app.dirty = true;
                            continue;
                        }

                        // Ctrl+C during streaming: cancel stream.
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.code == KeyCode::Char('c')
                            && app.streaming
                        {
                            app.renderer.finish();
                            app.abort_forwards();
                            chunk_rx = None;
                            app.streaming = false;
                            app.dirty = true;
                            continue;
                        }

                        match app.input.handle_key(key) {
                            InputAction::Submit(content) => {
                                if content.is_empty() {
                                    app.dirty = true;
                                    continue;
                                }
                                // Echo user input in chat.
                                app.renderer.buffer.push(ChatEntry::Text(vec![
                                    Line::raw(""),
                                    Line::from(Span::styled(
                                        format!(" {content} "),
                                        Style::new().bg(Color::Indexed(236)),
                                    )),
                                    Line::raw(""),
                                ]));
                                app.scroll = 0;

                                // Handle slash commands.
                                if content.starts_with('/') {
                                    match handle_slash(&content).await? {
                                        SlashResult::Handled => {}
                                        SlashResult::NotSlash => {
                                            send_or_queue(app, &mut chunk_rx, content);
                                        }
                                        SlashResult::Forward(cmd) => {
                                            send_or_queue(app, &mut chunk_rx, cmd);
                                        }
                                        SlashResult::Exit => return Ok(()),
                                        SlashResult::Resume => {
                                            // Temporarily leave fullscreen for console.
                                            crate::tui::teardown(terminal)?;
                                            let console = crate::cmd::console::Console;
                                            if let Ok((transport, info)) =
                                                crate::cmd::connect_default().await
                                                && let Ok(Some(_path)) =
                                                    console.run(transport, info).await
                                            {
                                                // Resume is informational only — conversations
                                                // are continuous per (agent, sender).
                                                app.renderer.buffer.push(ChatEntry::Text(vec![
                                                    Line::from(Span::styled(
                                                        "  Conversations are continuous — just keep chatting.",
                                                        Style::new().add_modifier(Modifier::DIM),
                                                    )),
                                                ]));
                                            }
                                            *terminal = crate::tui::setup()?;
                                        }
                                        SlashResult::Clear => {
                                            app.renderer.buffer.clear();
                                            app.renderer = MarkdownRenderer::new();
                                            app.chat_title.clear();
                                            // New conversation, so the preamble goes out again.
                                            app.peers_pending = !app.peers.is_empty();
                                            // Kill the current conversation so a new one is created.
                                            let conn_info = app.conn_info.clone();
                                            let agent = app.agent.clone();
                                            let sender = app.os_user.clone();
                                            tokio::spawn(async move {
                                                let _ = conn_info.kill_conversation(agent, sender).await;
                                            });
                                            app.renderer.buffer.push(ChatEntry::Text(vec![
                                                welcome_line(&app.agent, app.model_name.as_deref()),
                                            ]));
                                        }
                                    }
                                } else {
                                    send_or_queue(app, &mut chunk_rx, content);
                                }
                                app.dirty = true;
                            }
                            InputAction::Interrupt => {
                                if !app.streaming {
                                    app.dirty = true;
                                }
                            }
                            InputAction::Eof => {
                                if !app.streaming {
                                    return Ok(());
                                }
                            }
                            InputAction::Noop => {
                                app.dirty = true;
                            }
                        }
                    }
                    Some(Ok(Event::Resize(_, _))) => {
                        app.dirty = true;
                    }
                    Some(Err(_)) => break,
                    _ => {}
                }
            }

            // Branch 3: delegate fan-out progress.
            Some(update) = progress_rx.recv() => {
                handle_progress(update, app);
                app.dirty = true;
            }

            // Branch 4: render tick (animation).
            _ = tick.tick() => {
                app.frame_count += 1;
                if app.renderer.waiting || app.streaming {
                    app.dirty = true;
                }
            }
        }
    }
    Ok(())
}

fn send_or_queue(
    app: &mut App,
    chunk_rx: &mut Option<mpsc::UnboundedReceiver<Result<OutputChunk>>>,
    content: String,
) {
    if app.streaming {
        // Show queued indicator.
        let display = format!("  [queued] {content}");
        app.message_queue.push_back(content);
        app.renderer
            .buffer
            .push(ChatEntry::Text(vec![Line::from(Span::styled(
                display,
                Style::new().add_modifier(Modifier::DIM),
            ))]));
    } else {
        *chunk_rx = Some(start_stream(app, &content));
    }
}

fn start_stream(app: &mut App, content: &str) -> mpsc::UnboundedReceiver<Result<OutputChunk>> {
    // The daemon doesn't read the user's filesystem and doesn't know
    // what OS the client runs on. We render local context (environment +
    // Crab.md) on this side and prepend it to the user message.
    let mut prefix = String::new();
    prefix.push_str(&format!(
        "<environment>\nos: {}\n</environment>\n\n",
        std::env::consts::OS
    ));
    if app.peers_pending {
        prefix.push_str(&app.peers);
        app.peers_pending = false;
    }
    if let Some(instr) = std::env::current_dir()
        .ok()
        .and_then(|cwd| instructions::discover(&cwd))
    {
        prefix.push_str(&format!("<instructions>\n{instr}\n</instructions>\n\n"));
    }
    let content = format!("{prefix}{content}");
    let req = StreamMsg {
        agent: app.agent.clone(),
        content,
        sender: Some(app.os_user.clone()),
        tools: tools::client_tools(),
        ..Default::default()
    };
    app.streaming = true;
    app.renderer.start_waiting();
    client::spawn_stream(app.conn_info.clone(), req)
}

/// Fold a fan-out update into the chat buffer. `Started` seeds the task
/// list; each `Finished` settles one row in place.
fn handle_progress(update: delegate::Progress, app: &mut App) {
    match update {
        delegate::Progress::Started { call_id, agents } => {
            let tasks = agents
                .into_iter()
                .map(|agent| TaskProgress {
                    agent,
                    status: ToolStatus::Running,
                    detail: String::new(),
                })
                .collect();
            // These rows say everything the raw result JSON would, in a
            // form worth reading — don't print the blob underneath them.
            app.suppressed_results.insert(call_id.clone());
            app.renderer
                .buffer
                .push(ChatEntry::DelegateTasks { call_id, tasks });
        }
        delegate::Progress::Active {
            call_id,
            index,
            calls,
        } => {
            let detail = calls
                .iter()
                .map(|(name, args)| app.renderer.tool_label(name, args))
                .collect::<Vec<_>>()
                .join(", ");
            app.renderer
                .buffer
                .set_delegate_task_detail(&call_id, index, detail);
        }
        delegate::Progress::Finished {
            call_id,
            index,
            ok,
            detail,
        } => {
            app.renderer
                .buffer
                .finish_delegate_task(&call_id, index, ok, detail);
        }
    }
}

fn handle_chunk(chunk: OutputChunk, app: &mut App) {
    match chunk {
        OutputChunk::Text(text) => {
            app.renderer.push_text(&text);
        }
        OutputChunk::Thinking(text) => {
            app.renderer.push_thinking(&text);
        }
        OutputChunk::ThinkingEnd => {
            // Flush the thinking buffer immediately so it appears as a
            // discrete block instead of waiting for the next text delta.
            app.renderer.flush_thinking();
        }
        OutputChunk::ToolStart(calls) => {
            app.renderer.push_tool_start(&calls);
        }
        OutputChunk::ToolResult(id, output) => {
            if !app.suppressed_results.remove(&id) {
                app.renderer.push_tool_result(&output);
            }
        }
        OutputChunk::ToolDone(success) => {
            app.renderer.push_tool_done(success);
        }
        OutputChunk::AskUser {
            questions,
            conversation_id,
            call_id,
        } => {
            app.renderer.finish();
            app.ask_state = Some(AskState::new(&questions));
            app.ask_conversation_id = Some(conversation_id);
            app.ask_call_id = Some(call_id);
        }
        OutputChunk::ToolCallForward {
            conversation_id,
            call_id,
            name,
            arguments,
        } => {
            // The daemon forwarded a client-tool call. Dispatch locally and
            // send the result back on a fresh connection — same shape as
            // the ask-user reply path. `conversation_id` is an opaque
            // routing token; just echo it.
            let conn_info = app.conn_info.clone();
            let progress_tx = app.progress_tx.clone();
            let handle = tokio::spawn(async move {
                // Only what this client declares can arrive here. Anything
                // else is answered rather than dropped: an unanswered forward
                // is a hang until the timeout, not a fallback.
                let result = if name == "delegate" {
                    delegate::execute(&conn_info, &arguments, &call_id, progress_tx).await
                } else {
                    Err(format!("{name} is not a tool this client executes"))
                };
                let (output, is_error) = match result {
                    Ok(output) => (output, false),
                    Err(e) => (e, true),
                };
                let _ = conn_info
                    .reply_to_tool(conversation_id, call_id, output, is_error)
                    .await;
            });
            app.track_forward(handle);
        }
        // Boundary markers — the renderer infers transitions from delta
        // arrival, so Start markers are inert. ThinkingEnd above is the
        // exception because it lets us flush thinking eagerly.
        OutputChunk::TextStart | OutputChunk::TextEnd | OutputChunk::ThinkingStart => {}
    }
    // Auto-scroll to bottom on new content.
    app.scroll = 0;
}

// ── Drawing ──────────────────────────────────────────────────────

fn draw(frame: &mut ratatui::Frame, app: &App) {
    let input_height = app.input.height().min(frame.area().height / 3).max(3);

    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(input_height)])
        .split(frame.area());

    // ── Chat area ──
    draw_chat(frame, chunks[0], app);

    // ── Input box ──
    app.input
        .render(frame, chunks[1], &app.agent, &app.chat_title);

    // ── Ask modal overlay ──
    if let Some(ref ask) = app.ask_state {
        ask.draw(frame);
    }
}

fn draw_chat(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, app: &App) {
    let mut lines = app.renderer.buffer.lines(app.frame_count);

    // Append the partially-streamed current line.
    if let Some(current) = app.renderer.current_line() {
        lines.push(current);
    }

    // Waiting spinner.
    if app.renderer.waiting {
        let spinner_char = if app.frame_count % 30 < 15 {
            "⏺"
        } else {
            " "
        };
        lines.push(Line::from(Span::styled(
            spinner_char,
            Style::new().add_modifier(Modifier::DIM),
        )));
    }

    let total_lines = lines.len() as u16;
    let visible = area.height;

    // Compute scroll offset.  scroll=0 means "follow bottom".
    let max_scroll = total_lines.saturating_sub(visible);
    let scroll_offset = if app.scroll == 0 {
        max_scroll
    } else {
        max_scroll.saturating_sub(app.scroll as u16)
    };

    let paragraph = Paragraph::new(lines).scroll((scroll_offset, 0));
    frame.render_widget(paragraph, area);
}
