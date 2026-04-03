//! Interactive TUI for managing conversations.

use crate::repl::runner::Runner;
use crate::tui;
use anyhow::Result;
use clap::Args;
use crossterm::event::{KeyCode, KeyModifiers};
use events::EventEntry;
use futures_util::StreamExt;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Tabs},
};
use conversations::{ConversationView, render_conversation_view};
use std::collections::VecDeque;
use tokio::sync::mpsc;
use wcore::protocol::api::Client;
use wcore::protocol::message::AgentEventMsg;

mod events;
mod conversations;

use events::render_events;

/// Interactive console for conversations.
#[derive(Args, Debug)]
pub struct Console;

impl Console {
    /// Run the console. Returns a file path if the user selected a conversation to resume.
    pub async fn run(self, mut runner: Runner) -> Result<Option<std::path::PathBuf>> {
        // Spawn background event subscription task.
        let (event_tx, event_rx) = mpsc::unbounded_channel::<AgentEventMsg>();
        let conn_info = runner.conn_info.clone();
        tokio::spawn(async move {
            let Ok(mut sub_runner) = Runner::connect_from(&conn_info).await else {
                return;
            };
            let stream = sub_runner.subscribe_events();
            tokio::pin!(stream);
            while let Some(Ok(msg)) = stream.next().await {
                if event_tx.send(msg).is_err() {
                    break;
                }
            }
        });

        let mut terminal = tui::setup()?;

        // Fetch initial data from daemon.
        let daemon_conversations = runner.list_active_conversations().await.unwrap_or_default();
        let conversations = runner
            .list_conversations(String::new(), String::new())
            .await
            .unwrap_or_default();
        let mut conversation_view = ConversationView::default();
        conversation_view.refresh_identities(&conversations, &daemon_conversations);

        let mut state = ConsoleState {
            status: String::from("Ready"),
            runner,
            tab: Tab::Conversations,
            conversation_view,
            daemon_conversations,
            events: VecDeque::new(),
            event_rx,
            event_scroll: 0,
        };

        let mut idle_ticks: u8 = 0;
        let result = loop {
            // Drain pending events.
            while let Ok(msg) = state.event_rx.try_recv() {
                let timestamp = chrono::DateTime::parse_from_rfc3339(&msg.timestamp)
                    .map(|dt| {
                        dt.with_timezone(&chrono::Local)
                            .format("%H:%M:%S")
                            .to_string()
                    })
                    .unwrap_or_else(|_| chrono::Local::now().format("%H:%M:%S").to_string());
                state.events.push_back(EventEntry { timestamp, msg });
                if state.events.len() > 500 {
                    state.events.pop_front();
                }
            }

            terminal.draw(|frame| render(frame, &state))?;
            if let Some(key) = tui::poll_key()? {
                idle_ticks = 0;
                if let Some(result) = handle_key(key, &mut state).await? {
                    break result;
                }
            } else {
                idle_ticks = idle_ticks.saturating_add(1);
                // Refresh daemon data every ~2s (8 × 250ms poll).
                if idle_ticks >= 8 {
                    idle_ticks = 0;
                    let timeout = std::time::Duration::from_millis(500);
                    if let Ok(Ok(conversations)) =
                        tokio::time::timeout(timeout, state.runner.list_active_conversations()).await
                    {
                        state.daemon_conversations = conversations;
                        state.conversation_view.merge_daemon_data(&state.daemon_conversations);
                    }
                }
            }
        };

        tui::teardown(&mut terminal)?;
        Ok(result)
    }
}

// ── State ────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Conversations,
    Events,
}

pub(crate) struct ConsoleState {
    pub(crate) status: String,
    pub(crate) runner: Runner,
    tab: Tab,
    conversation_view: ConversationView,
    daemon_conversations: Vec<wcore::protocol::message::SessionInfo>,
    events: VecDeque<EventEntry>,
    event_rx: mpsc::UnboundedReceiver<AgentEventMsg>,
    event_scroll: usize,
}

// ── Key handling ────────────────────────────────────────────────────

/// Returns `Some(None)` to quit, `Some(Some(path))` to resume a conversation.
async fn handle_key(
    key: crossterm::event::KeyEvent,
    state: &mut ConsoleState,
) -> Result<Option<Option<std::path::PathBuf>>> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(Some(None));
    }

    match key.code {
        KeyCode::Char('q') => return Ok(Some(None)),
        KeyCode::Tab => {
            state.tab = match state.tab {
                Tab::Conversations => Tab::Events,
                Tab::Events => Tab::Conversations,
            };
        }
        _ => match state.tab {
            Tab::Conversations => {
                if let Some(path) = handle_conversations_key(key.code, state).await {
                    return Ok(Some(Some(path)));
                }
            }
            Tab::Events => handle_events_key(key.code, state),
        },
    }
    Ok(None)
}

async fn handle_conversations_key(
    code: KeyCode,
    state: &mut ConsoleState,
) -> Option<std::path::PathBuf> {
    match code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.conversation_view.move_up();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.conversation_view.move_down();
        }
        KeyCode::Enter => {
            // In conversation view: select and return the file path to resume.
            if let Some(path) = state.conversation_view.selected_file() {
                return Some(path);
            }
            // In identity view: drill down — fetch conversations for the selected identity.
            if let Some((agent, sender)) = state.conversation_view.selected_identity() {
                let agent = agent.to_string();
                let sender = sender.to_string();
                let timeout = std::time::Duration::from_millis(500);
                if let Ok(Ok(sessions)) =
                    tokio::time::timeout(timeout, state.runner.list_active_conversations()).await
                {
                    state.daemon_conversations = sessions;
                }
                let conversations = tokio::time::timeout(
                    timeout,
                    state
                        .runner
                        .list_conversations(agent.clone(), sender.clone()),
                )
                .await
                .ok()
                .and_then(|r| r.ok())
                .unwrap_or_default();
                state
                    .conversation_view
                    .enter(&conversations, &state.daemon_conversations);
            }
        }
        KeyCode::Char('d') => {
            // Delete the selected conversation.
            if let Some(path) = state.conversation_view.selected_file() {
                let _ = state
                    .runner
                    .delete_conversation(path.to_string_lossy().into_owned())
                    .await;
                state.status = "Deleted".into();
                // Refresh the conversation list.
                if let Some((agent, sender)) = state.conversation_view.current_identity() {
                    let timeout = std::time::Duration::from_millis(500);
                    let conversations = tokio::time::timeout(
                        timeout,
                        state
                            .runner
                            .list_conversations(agent.clone(), sender.clone()),
                    )
                    .await
                    .ok()
                    .and_then(|r| r.ok())
                    .unwrap_or_default();
                    state
                        .conversation_view
                        .enter(&conversations, &state.daemon_conversations);
                }
            }
        }
        KeyCode::Esc => {
            let timeout = std::time::Duration::from_millis(500);
            let conversations = tokio::time::timeout(
                timeout,
                state
                    .runner
                    .list_conversations(String::new(), String::new()),
            )
            .await
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or_default();
            state
                .conversation_view
                .back(&conversations, &state.daemon_conversations);
        }
        KeyCode::Char('r') => {
            let timeout = std::time::Duration::from_millis(500);
            if let Ok(Ok(sessions)) =
                tokio::time::timeout(timeout, state.runner.list_active_conversations()).await
            {
                state.daemon_conversations = sessions;
            }
            let conversations = tokio::time::timeout(
                timeout,
                state
                    .runner
                    .list_conversations(String::new(), String::new()),
            )
            .await
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or_default();
            state
                .conversation_view
                .refresh_identities(&conversations, &state.daemon_conversations);
            state.status = String::from("Refreshed");
        }
        _ => {}
    }
    None
}

fn handle_events_key(code: KeyCode, state: &mut ConsoleState) {
    match code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.event_scroll = state.event_scroll.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if !state.events.is_empty() {
                state.event_scroll = (state.event_scroll + 1).min(state.events.len() - 1);
            }
        }
        _ => {}
    }
}

// ── Render ──────────────────────────────────────────────────────────

const TAB_TITLES: &[&str] = &["Conversations", "Events"];

fn render(frame: &mut Frame, state: &ConsoleState) {
    let chunks = Layout::default()
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(frame.area());

    // Tab bar
    let tab_idx = match state.tab {
        Tab::Conversations => 0,
        Tab::Events => 1,
    };
    let tabs = Tabs::new(TAB_TITLES.iter().map(|t| Line::from(*t)))
        .select(tab_idx)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .divider(" | ");
    frame.render_widget(tabs, chunks[0]);

    // Content
    match state.tab {
        Tab::Conversations => render_conversation_view(frame, &state.conversation_view, chunks[1]),
        Tab::Events => {
            let events: Vec<&EventEntry> = state.events.iter().collect();
            render_events(frame, &events, state.event_scroll, chunks[1]);
        }
    }

    // Help bar
    render_help_bar(frame, state, chunks[2]);
}

fn render_help_bar(frame: &mut Frame, state: &ConsoleState, area: Rect) {
    let key_style = Style::default().fg(Color::Cyan);
    let mut spans = vec![Span::styled(" j/k ", key_style), Span::raw("Navigate  ")];

    if state.tab == Tab::Conversations {
        let in_conversations = matches!(state.conversation_view, ConversationView::Conversations { .. });
        if in_conversations {
            spans.extend([Span::styled("Esc ", key_style), Span::raw("Back  ")]);
        } else {
            spans.extend([Span::styled("Enter ", key_style), Span::raw("Open  ")]);
        }
        spans.extend([Span::styled("r ", key_style), Span::raw("Refresh  ")]);
    }

    spans.extend([
        Span::styled("Tab ", key_style),
        Span::raw("Switch  "),
        Span::styled("q ", key_style),
        Span::raw("Quit  "),
        Span::styled(
            format!(" {} ", state.status),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
