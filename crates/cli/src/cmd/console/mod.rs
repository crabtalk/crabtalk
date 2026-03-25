//! Interactive TUI for managing sessions.

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
use sessions::{SessionView, render_session_view};
use std::collections::VecDeque;
use tokio::sync::mpsc;
use wcore::protocol::message::AgentEventMsg;

mod events;
mod sessions;

use events::render_events;

/// Interactive console for sessions.
#[derive(Args, Debug)]
pub struct Console;

impl Console {
    pub async fn run(self, mut runner: Runner) -> Result<()> {
        // Spawn background event subscription task.
        let (event_tx, event_rx) = mpsc::unbounded_channel::<AgentEventMsg>();
        let socket_path = wcore::paths::SOCKET_PATH.to_path_buf();
        tokio::spawn(async move {
            let Ok(mut sub_runner) = Runner::connect(&socket_path).await else {
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

        // Fetch initial daemon session data.
        let daemon_sessions = runner.list_sessions().await.unwrap_or_default();
        let mut session_view = SessionView::default();
        session_view.refresh_identities(&daemon_sessions);

        let mut state = ConsoleState {
            status: String::from("Ready"),
            runner,
            tab: Tab::Sessions,
            session_view,
            daemon_sessions,
            events: VecDeque::new(),
            event_rx,
            event_scroll: 0,
        };

        let mut idle_ticks: u8 = 0;
        let result = loop {
            // Drain pending events.
            while let Ok(msg) = state.event_rx.try_recv() {
                let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
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
                    if let Ok(Ok(sessions)) =
                        tokio::time::timeout(timeout, state.runner.list_sessions()).await
                    {
                        state.daemon_sessions = sessions;
                        state.session_view.merge_daemon_data(&state.daemon_sessions);
                    }
                }
            }
        };

        tui::teardown(&mut terminal)?;
        result
    }
}

// ── State ────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Sessions,
    Events,
}

pub(crate) struct ConsoleState {
    pub(crate) status: String,
    pub(crate) runner: Runner,
    tab: Tab,
    session_view: SessionView,
    daemon_sessions: Vec<wcore::protocol::message::SessionInfo>,
    events: VecDeque<EventEntry>,
    event_rx: mpsc::UnboundedReceiver<AgentEventMsg>,
    event_scroll: usize,
}

// ── Key handling ────────────────────────────────────────────────────

async fn handle_key(
    key: crossterm::event::KeyEvent,
    state: &mut ConsoleState,
) -> Result<Option<Result<()>>> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(Some(Ok(())));
    }

    match key.code {
        KeyCode::Char('q') => return Ok(Some(Ok(()))),
        KeyCode::Tab => {
            state.tab = match state.tab {
                Tab::Sessions => Tab::Events,
                Tab::Events => Tab::Sessions,
            };
        }
        _ => match state.tab {
            Tab::Sessions => handle_sessions_key(key.code, state).await,
            Tab::Events => handle_events_key(key.code, state),
        },
    }
    Ok(None)
}

async fn handle_sessions_key(code: KeyCode, state: &mut ConsoleState) {
    match code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.session_view.move_up();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.session_view.move_down();
        }
        KeyCode::Enter => {
            // Refresh daemon data and drill down.
            let timeout = std::time::Duration::from_millis(500);
            if let Ok(Ok(sessions)) =
                tokio::time::timeout(timeout, state.runner.list_sessions()).await
            {
                state.daemon_sessions = sessions;
            }
            state.session_view.enter(&state.daemon_sessions);
        }
        KeyCode::Esc => {
            state.session_view.back(&state.daemon_sessions);
        }
        KeyCode::Char('r') => {
            let timeout = std::time::Duration::from_millis(500);
            if let Ok(Ok(sessions)) =
                tokio::time::timeout(timeout, state.runner.list_sessions()).await
            {
                state.daemon_sessions = sessions;
            }
            state
                .session_view
                .refresh_identities(&state.daemon_sessions);
            state.status = String::from("Refreshed");
        }
        _ => {}
    }
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

const TAB_TITLES: &[&str] = &["Sessions", "Events"];

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
        Tab::Sessions => 0,
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
        Tab::Sessions => render_session_view(frame, &state.session_view, chunks[1]),
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

    if state.tab == Tab::Sessions {
        let in_conversations = matches!(state.session_view, SessionView::Conversations { .. });
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
