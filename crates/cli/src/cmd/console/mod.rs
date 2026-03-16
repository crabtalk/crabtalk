//! Interactive TUI for managing sessions and tasks.

use crate::repl::runner::Runner;
use crate::tui;
use anyhow::Result;
use clap::Args;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Tabs},
};
use sessions::render_sessions;
use tasks::{handle_approve_input, render_tasks};
use wcore::protocol::message::{SessionInfo, TaskInfo};

mod sessions;
mod tasks;

/// Interactive console for sessions and tasks.
#[derive(Args, Debug)]
pub struct Console;

impl Console {
    pub async fn run(self, mut runner: Runner) -> Result<()> {
        let sessions = runner.list_sessions().await.unwrap_or_default();
        let tasks = runner.list_tasks().await.unwrap_or_default();

        let mut terminal = tui::setup()?;
        let mut state = ConsoleState {
            tab: Tab::Sessions,
            focus: Focus::List,
            sessions,
            tasks,
            selected: 0,
            cursor: 0,
            edit_buf: String::new(),
            status: String::from("Ready"),
            runner,
        };

        let result = loop {
            terminal.draw(|frame| render(frame, &state))?;
            if let Some(key) = tui::poll_key()?
                && let Some(result) = handle_key(key, &mut state).await?
            {
                break result;
            }
        };

        tui::teardown(&mut terminal)?;
        result
    }
}

// ── Tabs ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Sessions,
    Tasks,
}

const TAB_TITLES: &[&str] = &["Sessions", "Tasks"];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    List,
    Approve,
}

// ── State ────────────────────────────────────────────────────────────

pub(crate) struct ConsoleState {
    pub(crate) tab: Tab,
    pub(crate) focus: Focus,
    pub(crate) sessions: Vec<SessionInfo>,
    pub(crate) tasks: Vec<TaskInfo>,
    pub(crate) selected: usize,
    pub(crate) cursor: usize,
    pub(crate) edit_buf: String,
    pub(crate) status: String,
    pub(crate) runner: Runner,
}

impl ConsoleState {
    pub(crate) async fn refresh(&mut self) {
        match self.tab {
            Tab::Sessions => {
                self.sessions = self.runner.list_sessions().await.unwrap_or_default();
                if self.selected >= self.sessions.len() {
                    self.selected = self.sessions.len().saturating_sub(1);
                }
            }
            Tab::Tasks => {
                self.tasks = self.runner.list_tasks().await.unwrap_or_default();
                if self.selected >= self.tasks.len() {
                    self.selected = self.tasks.len().saturating_sub(1);
                }
            }
        }
    }

    fn list_len(&self) -> usize {
        match self.tab {
            Tab::Sessions => self.sessions.len(),
            Tab::Tasks => self.tasks.len(),
        }
    }
}

// ── Key handling ────────────────────────────────────────────────────

async fn handle_key(
    key: crossterm::event::KeyEvent,
    state: &mut ConsoleState,
) -> Result<Option<Result<()>>> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(Some(Ok(())));
    }

    if key.code == KeyCode::Tab && state.focus == Focus::List {
        state.tab = match state.tab {
            Tab::Sessions => Tab::Tasks,
            Tab::Tasks => Tab::Sessions,
        };
        state.selected = 0;
        state.refresh().await;
        return Ok(None);
    }

    match state.focus {
        Focus::List => handle_list(key, state).await,
        Focus::Approve => {
            handle_approve_input(key, state).await;
            Ok(None)
        }
    }
}

async fn handle_list(
    key: crossterm::event::KeyEvent,
    state: &mut ConsoleState,
) -> Result<Option<Result<()>>> {
    let len = state.list_len();
    match key.code {
        KeyCode::Char('q') => return Ok(Some(Ok(()))),
        KeyCode::Up | KeyCode::Char('k') => {
            state.selected = state.selected.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if len > 0 && state.selected < len - 1 {
                state.selected += 1;
            }
        }
        KeyCode::Char('r') => {
            state.refresh().await;
            state.status = String::from("Refreshed");
        }
        KeyCode::Char('d') | KeyCode::Delete => {
            match state.tab {
                Tab::Sessions => {
                    if let Some(s) = state.sessions.get(state.selected) {
                        let id = s.id;
                        match state.runner.kill_session(id).await {
                            Ok(true) => state.status = format!("Session {id} killed"),
                            Ok(false) => state.status = format!("Session {id} not found"),
                            Err(e) => state.status = format!("Error: {e}"),
                        }
                    }
                }
                Tab::Tasks => {
                    if let Some(t) = state.tasks.get(state.selected) {
                        let id = t.id;
                        match state.runner.kill_task(id).await {
                            Ok(true) => state.status = format!("Task {id} killed"),
                            Ok(false) => state.status = format!("Task {id} not found"),
                            Err(e) => state.status = format!("Error: {e}"),
                        }
                    }
                }
            }
            state.refresh().await;
        }
        KeyCode::Char('a') => {
            // Approve a blocked task.
            if state.tab == Tab::Tasks {
                if let Some(t) = state.tasks.get(state.selected)
                    && t.blocked_on.is_some()
                {
                    state.focus = Focus::Approve;
                    state.edit_buf.clear();
                    state.cursor = 0;
                } else {
                    state.status = String::from("Task is not blocked");
                }
            }
        }
        _ => {}
    }
    Ok(None)
}

// ── Rendering ───────────────────────────────────────────────────────

fn render(frame: &mut Frame, state: &ConsoleState) {
    let area = frame.area();

    let outer = Block::default()
        .title(" Walrus Console ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let vert = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(2),
    ])
    .split(inner);

    // Tab bar.
    let tab_idx = match state.tab {
        Tab::Sessions => 0,
        Tab::Tasks => 1,
    };
    let tabs = Tabs::new(TAB_TITLES.iter().map(|t| Line::from(*t)))
        .select(tab_idx)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .divider(" | ");
    frame.render_widget(tabs, vert[0]);

    match state.tab {
        Tab::Sessions => render_sessions(frame, state, vert[1]),
        Tab::Tasks => render_tasks(frame, state, vert[1]),
    }

    render_status(frame, state, vert[2]);
}

fn render_status(frame: &mut Frame, state: &ConsoleState, area: ratatui::layout::Rect) {
    let help = match state.focus {
        Focus::Approve => Line::from(vec![
            Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("Send  "),
            Span::styled("Esc ", Style::default().fg(Color::Cyan)),
            Span::raw("Cancel  "),
            Span::styled("| ", Style::default().fg(Color::DarkGray)),
            Span::styled(&state.status, Style::default().fg(Color::Green)),
        ]),
        Focus::List => {
            let mut spans = vec![
                Span::styled(" Tab ", Style::default().fg(Color::Cyan)),
                Span::raw("Switch  "),
                Span::styled("r ", Style::default().fg(Color::Cyan)),
                Span::raw("Refresh  "),
                Span::styled("d ", Style::default().fg(Color::Cyan)),
                Span::raw("Kill  "),
            ];
            if state.tab == Tab::Tasks {
                spans.push(Span::styled("a ", Style::default().fg(Color::Cyan)));
                spans.push(Span::raw("Approve  "));
            }
            spans.push(Span::styled("q ", Style::default().fg(Color::Cyan)));
            spans.push(Span::raw("Quit  "));
            spans.push(Span::styled("| ", Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(
                &state.status,
                Style::default().fg(Color::Green),
            ));
            Line::from(spans)
        }
    };
    frame.render_widget(Paragraph::new(help), area);
}
