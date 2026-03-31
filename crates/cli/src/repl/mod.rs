//! Full-screen interactive chat REPL with concurrent input and streaming.

use crate::repl::{
    ask::{AskAction, AskState},
    chat::ChatEntry,
    command::{SlashResult, handle_slash},
    input::{History, InputAction, InputState},
    render::MarkdownRenderer,
    runner::{ConnectionInfo, OutputChunk, Runner, send_reply},
};
use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyModifiers};
use futures_util::StreamExt;
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use std::{collections::VecDeque, path::PathBuf, pin::pin, time::Duration};
use tokio::sync::mpsc;

mod ask;
pub mod chat;
pub mod command;
pub mod input;
pub mod render;
pub mod runner;

/// Interactive chat REPL.
pub struct ChatRepl {
    runner: Runner,
    agent: String,
    history_path: Option<PathBuf>,
    history: History,
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
            history_path,
            history,
        })
    }

    /// Resume a specific session file in the interactive REPL.
    pub async fn resume(&mut self, path: PathBuf) -> Result<()> {
        let title = wcore::Session::load_context(&path)
            .ok()
            .map(|(meta, _)| meta.title)
            .unwrap_or_default();
        let resume_file = Some(path.to_string_lossy().into_owned());
        self.run_inner(title, resume_file).await
    }

    /// Run the full-screen interactive REPL loop.
    pub async fn run(&mut self) -> Result<()> {
        let os_user = std::env::var("USER").unwrap_or_else(|_| "user".into());
        let chat_title =
            wcore::find_latest_session(&wcore::paths::SESSIONS_DIR, &self.agent, &os_user)
                .and_then(|path| wcore::Session::load_context(&path).ok())
                .map(|(meta, _)| meta.title)
                .unwrap_or_default();
        self.run_inner(chat_title, None).await
    }

    async fn run_inner(&mut self, chat_title: String, resume_file: Option<String>) -> Result<()> {
        let model = self.fetch_model_name().await;
        let conn_info = self.runner.conn_info().clone();
        let os_user = std::env::var("USER").unwrap_or_else(|_| "user".into());

        let skill_names = self.runner.list_skills().await.unwrap_or_default();
        let history = std::mem::take(&mut self.history);
        let mut app = App {
            renderer: MarkdownRenderer::new(),
            input: InputState::new(history, skill_names),
            scroll: 0,
            message_queue: VecDeque::new(),
            agent: self.agent.clone(),
            chat_title,
            new_chat: false,
            resume_file,
            dirty: true,
            frame_count: 0,
            skip_tool_result: 0,
            streaming: false,
            conn_info,
            os_user,
            model_name: model,
            ask_state: None,
            ask_session: None,
        };

        // Push welcome banner as first chat entry.
        app.renderer.buffer.push(ChatEntry::Text(vec![welcome_line(
            &app.agent,
            app.model_name.as_deref(),
        )]));

        let mut terminal = crate::tui::setup()?;
        let result = run_event_loop(&mut terminal, &mut app).await;

        crate::tui::teardown(&mut terminal)?;

        // Save history back.
        self.history = std::mem::take(&mut app.input.history);
        self.save_history();

        result
    }

    async fn fetch_model_name(&mut self) -> Option<String> {
        let config_json = self.runner.get_config().await.ok()?;
        let val: serde_json::Value = serde_json::from_str(&config_json).ok()?;
        val.get("system")?
            .get("crab")?
            .get("model")?
            .as_str()
            .map(|s| s.to_string())
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
    new_chat: bool,
    resume_file: Option<String>,
    dirty: bool,
    frame_count: u64,
    skip_tool_result: u32,
    streaming: bool,
    conn_info: ConnectionInfo,
    os_user: String,
    model_name: Option<String>,
    /// Active ask-user modal (if any).
    ask_state: Option<AskState>,
    /// Session ID for the pending ask reply.
    ask_session: Option<u64>,
}

// ── Event loop ───────────────────────────────────────────────────

async fn run_event_loop(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
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
                        // Pick up title once (daemon generates it after first exchange).
                        if app.chat_title.is_empty()
                            && let Some(path) = wcore::find_latest_session(
                                &wcore::paths::SESSIONS_DIR, &app.agent, &app.os_user,
                            )
                            && let Ok((meta, _)) = wcore::Session::load_context(&path)
                            && !meta.title.is_empty()
                        {
                            app.chat_title = meta.title;
                        }
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
                                    app.ask_session = None;
                                }
                                AskAction::Submitted(answers) => {
                                    let reply = serde_json::to_string(&answers).unwrap_or_default();
                                    if let Some(session) = app.ask_session.take() {
                                        let conn_info = app.conn_info.clone();
                                        tokio::spawn(async move {
                                            let _ = send_reply(&conn_info, session, reply).await;
                                        });
                                    }
                                    app.ask_state = None;
                                    app.skip_tool_result += 1;
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
                                        format!(" {} ", &content),
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
                                            if let Ok(runner) = crate::cmd::connect_default().await
                                                && let Ok(Some(path)) = console.run(runner).await
                                            {
                                                // Load title from the resumed session.
                                                if let Ok((meta, _)) = wcore::Session::load_context(&path) {
                                                    if !meta.title.is_empty() {
                                                        app.chat_title.clone_from(&meta.title);
                                                    }
                                                    app.renderer.buffer.push(ChatEntry::Text(vec![
                                                        Line::from(Span::styled(
                                                            format!("  Resumed: {}", if meta.title.is_empty() {
                                                                path.file_name().unwrap_or_default().to_string_lossy().into_owned()
                                                            } else {
                                                                meta.title
                                                            }),
                                                            Style::new().add_modifier(Modifier::DIM),
                                                        )),
                                                    ]));
                                                }
                                                app.resume_file = Some(path.to_string_lossy().into_owned());
                                            }
                                            *terminal = crate::tui::setup()?;
                                        }
                                        SlashResult::Clear => {
                                            app.renderer.buffer.clear();
                                            app.renderer = MarkdownRenderer::new();
                                            app.new_chat = true;
                                            app.chat_title.clear();
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

            // Branch 3: render tick (animation).
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
        let display = format!("  ⏳ queued: {}", &content);
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
    let (tx, rx) = mpsc::unbounded_channel();
    let conn_info = app.conn_info.clone();
    let agent = app.agent.clone();
    let content = content.to_string();
    let sender = Some(app.os_user.clone());
    let new_chat = app.new_chat;
    let resume_file = app.resume_file.take();

    app.streaming = true;
    app.renderer.start_waiting();
    app.new_chat = false;

    tokio::spawn(async move {
        let runner = Runner::connect_from(&conn_info).await;
        let mut runner = match runner {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        let cwd = std::env::current_dir().ok();
        let stream = runner.stream(
            &agent,
            &content,
            cwd.as_deref(),
            new_chat,
            resume_file,
            sender,
        );
        let mut stream = pin!(stream);
        while let Some(chunk) = stream.next().await {
            if tx.send(chunk).is_err() {
                break;
            }
        }
    });

    rx
}

fn handle_chunk(chunk: OutputChunk, app: &mut App) {
    match chunk {
        OutputChunk::Text(text) => {
            app.renderer.push_text(&text);
        }
        OutputChunk::Thinking(text) => {
            app.renderer.push_thinking(&text);
        }
        OutputChunk::ToolStart(calls) => {
            app.renderer.push_tool_start(&calls);
        }
        OutputChunk::ToolResult(_id, output) => {
            if app.skip_tool_result > 0 {
                app.skip_tool_result -= 1;
            } else {
                app.renderer.push_tool_result(&output);
            }
        }
        OutputChunk::ToolDone(success) => {
            app.renderer.push_tool_done(success);
        }
        OutputChunk::AskUser { questions, session } => {
            app.renderer.finish();
            app.ask_state = Some(AskState::new(&questions));
            app.ask_session = Some(session);
        }
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

// ── Legacy helpers kept for other callers (e.g. hub.rs) ──────────

/// Consume a stream of output chunks (legacy — used by hub.rs).
pub(crate) async fn stream_to_terminal(
    stream: impl futures_core::Stream<Item = Result<OutputChunk>>,
    _conn_info: &ConnectionInfo,
) -> Result<()> {
    use std::io::Write;
    let mut stream = pin!(stream);
    let mut renderer = MarkdownRenderer::new();
    renderer.start_waiting();

    while let Some(chunk) = stream.next().await {
        match chunk? {
            OutputChunk::Text(text) => renderer.push_text(&text),
            OutputChunk::Thinking(text) => renderer.push_thinking(&text),
            OutputChunk::ToolStart(calls) => renderer.push_tool_start(&calls),
            OutputChunk::ToolResult(_id, output) => renderer.push_tool_result(&output),
            OutputChunk::ToolDone(success) => renderer.push_tool_done(success),
            OutputChunk::AskUser { .. } => {}
        }
    }
    renderer.finish();

    // Dump buffer to stdout for legacy callers.
    let lines = renderer.buffer.lines(0);
    let mut stdout = std::io::stdout().lock();
    for line in &lines {
        for span in &line.spans {
            write!(stdout, "{}", span.content)?;
        }
        writeln!(stdout)?;
    }
    stdout.flush()?;
    Ok(())
}
