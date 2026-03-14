//! Interactive TUI for configuring gateway tokens.

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use gateway::GatewayType;
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use std::{io::Stdout, time::Duration};
use toml_edit::{DocumentMut, Item, Table};

/// Configure channel tokens interactively.
#[derive(clap::Args, Debug)]
pub struct Auth;

/// Two fixed rows: Telegram (0) and Discord (1).
const PLATFORM_NAMES: [&str; 2] = ["Telegram", "Discord"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    List,
    Editing,
}

struct AuthState {
    focus: Focus,
    /// 0 = Telegram, 1 = Discord.
    selected: usize,
    cursor: usize,
    tokens: [String; 2],
    status: String,
}

impl AuthState {
    fn load() -> Result<Self> {
        let config_path = wcore::paths::CONFIG_DIR.join("walrus.toml");
        let mut tokens = [String::new(), String::new()];

        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .with_context(|| format!("cannot read {}", config_path.display()))?;
            let doc: DocumentMut = content
                .parse()
                .with_context(|| format!("invalid TOML in {}", config_path.display()))?;

            if let Some(channel) = doc.get("channel").and_then(|c| c.as_table()) {
                if let Some(tg) = channel.get("telegram").and_then(|t| t.as_table()) {
                    tokens[0] = tg
                        .get("token")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                }
                if let Some(dc) = channel.get("discord").and_then(|t| t.as_table()) {
                    tokens[1] = dc
                        .get("token")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                }
            }
        }

        Ok(Self {
            focus: Focus::List,
            selected: 0,
            cursor: 0,
            tokens,
            status: String::from("Ready"),
        })
    }

    fn save(&mut self) -> Result<()> {
        let config_path = wcore::paths::CONFIG_DIR.join("walrus.toml");
        std::fs::create_dir_all(&*wcore::paths::CONFIG_DIR)
            .with_context(|| format!("cannot create {}", wcore::paths::CONFIG_DIR.display()))?;

        let content = if config_path.exists() {
            std::fs::read_to_string(&config_path)
                .with_context(|| format!("cannot read {}", config_path.display()))?
        } else {
            String::new()
        };

        let mut doc: DocumentMut = content
            .parse()
            .with_context(|| format!("invalid TOML in {}", config_path.display()))?;

        doc.remove("channel");

        let mut channel_table = Table::new();
        if !self.tokens[0].is_empty() {
            let mut tg = Table::new();
            tg.insert("token", toml_edit::value(&self.tokens[0]));
            channel_table.insert("telegram", Item::Table(tg));
        }
        if !self.tokens[1].is_empty() {
            let mut dc = Table::new();
            dc.insert("token", toml_edit::value(&self.tokens[1]));
            channel_table.insert("discord", Item::Table(dc));
        }
        if !channel_table.is_empty() {
            doc.insert("channel", Item::Table(channel_table));
        }

        std::fs::write(&config_path, doc.to_string())
            .with_context(|| format!("failed to write {}", config_path.display()))?;

        self.status = String::from("Saved!");
        Ok(())
    }

    fn current_token(&self) -> &str {
        &self.tokens[self.selected]
    }

    fn current_token_mut(&mut self) -> &mut String {
        &mut self.tokens[self.selected]
    }
}

/// Convert a char index to a byte offset within a string.
fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

impl Auth {
    pub fn run(self) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen)?;

        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
            original_hook(info);
        }));

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        let mut state = AuthState::load()?;
        let result = run_loop(&mut terminal, &mut state);

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        result
    }
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    state: &mut AuthState,
) -> Result<()> {
    loop {
        terminal.draw(|frame| render(frame, state))?;
        if event::poll(Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
            && handle_key(key, state)?
        {
            return Ok(());
        }
    }
}

/// Returns true if the TUI should quit.
fn handle_key(key: event::KeyEvent, state: &mut AuthState) -> Result<bool> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
        if let Err(e) = state.save() {
            state.status = format!("Error: {e}");
        }
        return Ok(false);
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(true);
    }

    match state.focus {
        Focus::List => handle_list_key(key, state),
        Focus::Editing => Ok(handle_editing(key, state)),
    }
}

fn handle_list_key(key: event::KeyEvent, state: &mut AuthState) -> Result<bool> {
    match key.code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Up | KeyCode::Char('k') => {
            state.selected = state.selected.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if state.selected < PLATFORM_NAMES.len() - 1 {
                state.selected += 1;
            }
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            state.focus = Focus::Editing;
            state.cursor = state.current_token().chars().count();
        }
        KeyCode::Tab => {
            state.selected = (state.selected + 1) % PLATFORM_NAMES.len();
        }
        KeyCode::Char('x') | KeyCode::Delete => {
            state.tokens[state.selected].clear();
        }
        _ => {}
    }
    Ok(false)
}

/// Returns true if the TUI should quit.
fn handle_editing(key: event::KeyEvent, state: &mut AuthState) -> bool {
    let cursor = state.cursor;
    let val = state.current_token_mut();

    match key.code {
        KeyCode::Esc | KeyCode::Enter => {
            state.focus = Focus::List;
        }
        KeyCode::Backspace => {
            if cursor > 0 {
                let start = char_to_byte(val, cursor - 1);
                let end = char_to_byte(val, cursor);
                val.drain(start..end);
                state.cursor -= 1;
            }
        }
        KeyCode::Delete => {
            let char_count = val.chars().count();
            if cursor < char_count {
                let start = char_to_byte(val, cursor);
                let end = char_to_byte(val, cursor + 1);
                val.drain(start..end);
            }
        }
        KeyCode::Left => {
            state.cursor = cursor.saturating_sub(1);
        }
        KeyCode::Right => {
            let char_count = val.chars().count();
            if cursor < char_count {
                state.cursor = cursor + 1;
            }
        }
        KeyCode::Home => {
            state.cursor = 0;
        }
        KeyCode::End => {
            state.cursor = val.chars().count();
        }
        KeyCode::Char(c) => {
            let byte_pos = char_to_byte(val, cursor);
            val.insert(byte_pos, c);
            state.cursor = cursor + 1;
        }
        _ => {}
    }
    false
}

// ── Rendering ────────────────────────────────────────────────────────

fn render(frame: &mut Frame, state: &AuthState) {
    let area = frame.area();

    let outer = Block::default()
        .title(" Walrus Auth ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let vert = Layout::vertical([Constraint::Min(4), Constraint::Length(2)]).split(inner);
    let content_area = vert[0];
    let status_area = vert[1];

    let horiz = Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(content_area);
    let list_area = horiz[0];
    let field_area = horiz[1];

    render_list(frame, state, list_area);
    render_fields(frame, state, field_area);
    render_status(frame, state, status_area);
}

fn render_list(frame: &mut Frame, state: &AuthState, area: Rect) {
    let block = Block::default()
        .title(" Channels ")
        .borders(Borders::ALL)
        .border_style(if state.focus == Focus::List {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
        });

    let lines: Vec<Line> = PLATFORM_NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let marker = if i == state.selected { "> " } else { "  " };
            let configured = if state.tokens[i].is_empty() { "" } else { " *" };
            let text = format!("{marker}{name}{configured}");
            let style = if i == state.selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            Line::from(Span::styled(text, style))
        })
        .collect();

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn render_fields(frame: &mut Frame, state: &AuthState, area: Rect) {
    let name = PLATFORM_NAMES[state.selected];
    let token = state.current_token();
    let channel_type = GatewayType::VARIANTS[state.selected];

    let block = Block::default()
        .title(format!(" {name} "))
        .borders(Borders::ALL)
        .border_style(if state.focus == Focus::Editing {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
        });
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let label_span = Span::styled(
        "     Token: ",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );

    let line = if state.focus == Focus::Editing {
        let byte_pos = char_to_byte(token, state.cursor);
        let mut s = token.to_owned();
        s.insert(byte_pos, '|');
        Line::from(vec![
            label_span,
            Span::styled(s, Style::default().fg(Color::Green)),
        ])
    } else if token.is_empty() {
        Line::from(vec![
            label_span,
            Span::styled(
                channel_type.token_hint(),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
        ])
    } else {
        Line::from(vec![
            label_span,
            Span::styled(mask_token(token), Style::default().fg(Color::White)),
        ])
    };

    frame.render_widget(Paragraph::new(line), inner);
}

fn render_status(frame: &mut Frame, state: &AuthState, area: Rect) {
    let help = Line::from(vec![
        Span::styled(" Ctrl+S ", Style::default().fg(Color::Cyan)),
        Span::raw("Save  "),
        Span::styled("Enter ", Style::default().fg(Color::Cyan)),
        Span::raw("Edit  "),
        Span::styled("x ", Style::default().fg(Color::Cyan)),
        Span::raw("Clear  "),
        Span::styled("Esc ", Style::default().fg(Color::Cyan)),
        Span::raw("Back  "),
        Span::styled("q ", Style::default().fg(Color::Cyan)),
        Span::raw("Quit  "),
        Span::styled("| ", Style::default().fg(Color::DarkGray)),
        Span::styled(&state.status, Style::default().fg(Color::Green)),
    ]);
    frame.render_widget(Paragraph::new(help), area);
}

/// Mask a token for display — show first 4 and last 4 ASCII chars.
fn mask_token(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() <= 8 {
        "*".repeat(chars.len())
    } else {
        let head: String = chars[..4].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}{}{tail}", "*".repeat(chars.len() - 8))
    }
}
