//! Inline ratatui TUI for batch ask-user questions.
//!
//! All questions share a single block. Headers appear as tabs at the top;
//! Tab/Shift+Tab switches between them. A final "Submit" tab confirms.

use crate::tui;
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal, TerminalOptions, Viewport,
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Tabs},
};
use std::collections::{BTreeSet, HashMap};
use wcore::protocol::message::AskQuestion;

/// Run an inline TUI that presents questions in a tabbed block.
///
/// Returns a map of question text → selected label(s).
pub fn run_ask_inline(questions: &[AskQuestion]) -> Result<HashMap<String, String>> {
    let mut state = AskState::new(questions);
    let height = state.viewport_height();

    enable_raw_mode()?;
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(std::io::stdout()),
        TerminalOptions {
            viewport: Viewport::Inline(height as u16),
        },
    )?;

    let result = event_loop(&mut terminal, &mut state);

    // The inline viewport's last draw leaves the cursor at the bottom
    // of the rendered area. Just move one line past it for clean spacing.
    disable_raw_mode()?;
    println!();
    println!();

    result
}

// ── State ────────────────────────────────────────────────────────────

struct AskState {
    questions: Vec<QuestionState>,
    /// Index into tabs: 0..questions.len() = question tabs,
    /// questions.len() = Submit tab.
    focused: usize,
    mode: InputMode,
    input_buf: String,
    input_cursor: usize,
}

struct QuestionState {
    question: AskQuestion,
    selected: BTreeSet<usize>,
    cursor: usize,
    other_text: Option<String>,
}

#[derive(PartialEq)]
enum InputMode {
    Normal,
    TextInput,
}

impl AskState {
    fn new(questions: &[AskQuestion]) -> Self {
        Self {
            questions: questions.iter().map(QuestionState::new).collect(),
            focused: 0,
            mode: InputMode::Normal,
            input_buf: String::new(),
            input_cursor: 0,
        }
    }

    /// Height for the inline viewport: tab bar(1) + border(2) + question(1)
    /// + gap(1) + max options across all questions + other(1) + status(1).
    fn viewport_height(&self) -> usize {
        let max_opts = self
            .questions
            .iter()
            .map(|q| q.question.options.len())
            .max()
            .unwrap_or(0);
        // tab_bar(1) + top_border(1) + question_text(1) + gap(1) + options
        // + other(1) + bottom_border(1) + status(1)
        1 + 1 + 1 + 1 + max_opts + 1 + 1 + 1
    }

    /// Total tab count including the Submit tab.
    fn tab_count(&self) -> usize {
        self.questions.len() + 1
    }

    fn on_submit_tab(&self) -> bool {
        self.focused == self.questions.len()
    }

    fn focused_q(&self) -> &QuestionState {
        &self.questions[self.focused]
    }

    fn focused_q_mut(&mut self) -> &mut QuestionState {
        &mut self.questions[self.focused]
    }

    /// Number of items including the "Other" entry.
    fn item_count(&self) -> usize {
        self.focused_q().question.options.len() + 1
    }

    fn other_idx(&self) -> usize {
        self.focused_q().question.options.len()
    }

    /// Save current input buffer to the focused question's other_text.
    fn commit_input(&mut self) {
        let text = std::mem::take(&mut self.input_buf);
        self.input_cursor = 0;
        self.focused_q_mut().other_text = Some(text);
        self.mode = InputMode::Normal;
    }

    /// Advance to the next tab.
    fn advance(&mut self) {
        let count = self.tab_count();
        self.focused = (self.focused + 1).min(count - 1);
    }

    /// For single-select questions, cursor position is the selection.
    /// When cursor lands on "Other", enter text input mode directly.
    fn auto_select(&mut self) {
        let qs = &mut self.questions[self.focused];
        if qs.question.multi_select {
            return;
        }
        let cursor = qs.cursor;
        let other = qs.question.options.len();
        qs.selected.clear();
        if cursor < other {
            qs.selected.insert(cursor);
            self.mode = InputMode::Normal;
        } else {
            // Cursor on "Other": open text input directly.
            let existing = qs.other_text.clone().unwrap_or_default();
            self.input_buf = existing;
            self.input_cursor = self.input_buf.chars().count();
            self.mode = InputMode::TextInput;
        }
    }

    /// Build the answer map from current selections.
    fn answers(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for qs in &self.questions {
            let key = qs.question.question.clone();
            if let Some(ref text) = qs.other_text
                && (qs.question.multi_select || qs.selected.is_empty())
            {
                map.insert(key, text.clone());
            } else if qs.question.multi_select {
                let labels: Vec<&str> = qs
                    .selected
                    .iter()
                    .filter_map(|&i| qs.question.options.get(i).map(|o| o.label.as_str()))
                    .collect();
                map.insert(key, labels.join(", "));
            } else if let Some(&i) = qs.selected.iter().next()
                && let Some(opt) = qs.question.options.get(i)
            {
                map.insert(key, opt.label.clone());
            }
        }
        map
    }
}

impl QuestionState {
    fn new(q: &AskQuestion) -> Self {
        // Filter out any "Other" option — the TUI provides its own.
        let mut question = q.clone();
        question
            .options
            .retain(|o| !o.label.eq_ignore_ascii_case("other"));
        Self {
            question,
            selected: BTreeSet::new(),
            cursor: 0,
            other_text: None,
        }
    }
}

// ── Event loop ───────────────────────────────────────────────────────

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    state: &mut AskState,
) -> Result<HashMap<String, String>> {
    loop {
        terminal.draw(|frame| draw(frame, state))?;
        if !event::poll(std::time::Duration::from_millis(250))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            anyhow::bail!("cancelled");
        }

        // Text input mode — typing into the "Other" field.
        if state.mode == InputMode::TextInput {
            match key.code {
                KeyCode::Enter | KeyCode::Tab => {
                    state.commit_input();
                    state.advance();
                }
                KeyCode::Esc => {
                    state.input_buf.clear();
                    state.input_cursor = 0;
                    state.mode = InputMode::Normal;
                }
                KeyCode::Up => {
                    state.commit_input();
                    let q = state.focused_q_mut();
                    q.cursor = q.cursor.saturating_sub(1);
                    state.auto_select();
                }
                KeyCode::Down => {
                    state.commit_input();
                    let max = state.item_count().saturating_sub(1);
                    let q = state.focused_q_mut();
                    q.cursor = (q.cursor + 1).min(max);
                    state.auto_select();
                }
                code => {
                    tui::handle_text_input(code, &mut state.input_buf, &mut state.input_cursor);
                }
            }
            continue;
        }

        // Submit tab: Enter submits, everything else navigates away.
        if state.on_submit_tab() {
            match key.code {
                KeyCode::Esc => anyhow::bail!("cancelled"),
                KeyCode::Enter => return Ok(state.answers()),
                KeyCode::Tab => {
                    state.focused = 0;
                }
                KeyCode::BackTab => {
                    state.focused = state.questions.len().saturating_sub(1);
                }
                _ => {}
            }
            continue;
        }

        // Normal mode on a question tab.
        match key.code {
            KeyCode::Esc => anyhow::bail!("cancelled"),
            KeyCode::Enter | KeyCode::Tab => {
                state.advance();
            }
            KeyCode::Up => {
                let q = state.focused_q_mut();
                q.cursor = q.cursor.saturating_sub(1);
                state.auto_select();
            }
            KeyCode::Down => {
                let max = state.item_count().saturating_sub(1);
                let q = state.focused_q_mut();
                q.cursor = (q.cursor + 1).min(max);
                state.auto_select();
            }
            KeyCode::BackTab => {
                let count = state.tab_count();
                state.focused = (state.focused + count - 1) % count;
            }
            KeyCode::Char(' ') => {
                let other = state.other_idx();
                let cursor = state.focused_q().cursor;
                if cursor == other {
                    let existing = state.focused_q().other_text.clone().unwrap_or_default();
                    state.input_buf = existing;
                    state.input_cursor = state.input_buf.chars().count();
                    state.mode = InputMode::TextInput;
                } else if state.focused_q().question.multi_select {
                    let q = state.focused_q_mut();
                    q.other_text = None;
                    if q.selected.contains(&cursor) {
                        q.selected.remove(&cursor);
                    } else {
                        q.selected.insert(cursor);
                    }
                }
            }
            _ => {}
        }
    }
}

// ── Rendering ────────────────────────────────────────────────────────

fn draw(frame: &mut ratatui::Frame, state: &AskState) {
    let area = frame.area();

    // Layout: tab bar + content block + status bar.
    let chunks = Layout::default()
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    // Tab bar: question headers + "Submit".
    let mut titles: Vec<Line> = state
        .questions
        .iter()
        .map(|q| Line::from(q.question.header.clone()))
        .collect();
    titles.push(Line::from("Submit"));
    let tabs = Tabs::new(titles)
        .select(state.focused)
        .highlight_style(
            Style::default()
                .fg(Color::Rgb(215, 119, 87))
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().fg(Color::DarkGray))
        .divider(" | ");
    frame.render_widget(tabs, chunks[0]);

    // Content block.
    if state.on_submit_tab() {
        draw_submit(frame, state, chunks[1]);
    } else {
        draw_content(frame, state, chunks[1]);
    }

    // Status bar.
    draw_status(frame, state, chunks[2]);
}

fn draw_submit(frame: &mut ratatui::Frame, state: &AskState, area: ratatui::layout::Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(136, 136, 136)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    // Show a summary of answers.
    let mut lines = vec![
        Line::from(Span::styled(
            "Review your answers:",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for qs in &state.questions {
        let answer = if let Some(ref text) = qs.other_text
            && (qs.question.multi_select || qs.selected.is_empty())
        {
            format!("\"{}\"", text)
        } else if qs.selected.is_empty() {
            "(no selection)".to_string()
        } else {
            qs.selected
                .iter()
                .filter_map(|&i| qs.question.options.get(i).map(|o| o.label.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {} ", qs.question.header),
                Style::default().fg(Color::Rgb(215, 119, 87)),
            ),
            Span::styled(answer, Style::default().fg(Color::Cyan)),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_content(frame: &mut ratatui::Frame, state: &AskState, area: ratatui::layout::Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(136, 136, 136)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    let qs = state.focused_q();
    let multi = qs.question.multi_select;

    // Question text + gap line.
    let mut lines = vec![
        Line::from(Span::styled(
            &qs.question.question,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    // Options.
    for (i, opt) in qs.question.options.iter().enumerate() {
        let is_cursor = qs.cursor == i;
        let is_selected = qs.selected.contains(&i);
        let prefix = option_prefix(multi, is_cursor, is_selected);
        let label = if opt.description.is_empty() {
            opt.label.clone()
        } else {
            format!("{} — {}", opt.label, opt.description)
        };
        let style = if is_cursor && !multi {
            Style::default()
                .fg(Color::Rgb(215, 119, 87))
                .add_modifier(Modifier::BOLD)
        } else if is_cursor {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else if is_selected {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(format!("{prefix}{label}"), style)));
    }

    // "Other" option.
    let other_idx = qs.question.options.len();
    let is_cursor = qs.cursor == other_idx;
    // In single-select, Other is only "selected" when no regular option is.
    let has_other = qs.other_text.is_some() && (multi || qs.selected.is_empty());
    let prefix = option_prefix(multi, is_cursor, has_other);
    // Show live input inline on the "Other:" line.
    let other_text = if state.mode == InputMode::TextInput && is_cursor {
        &state.input_buf
    } else {
        match &qs.other_text {
            Some(t) => t.as_str(),
            None => "",
        }
    };
    let other_label = format!("{prefix}Other: {other_text}");
    let style = if is_cursor {
        Style::default()
            .fg(Color::Rgb(215, 119, 87))
            .add_modifier(Modifier::BOLD)
    } else if has_other {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    lines.push(Line::from(Span::styled(other_label, style)));

    frame.render_widget(Paragraph::new(lines), inner);
}

fn option_prefix(multi: bool, is_cursor: bool, is_selected: bool) -> &'static str {
    match (multi, is_cursor, is_selected) {
        (true, true, true) => "  [x] ",
        (true, true, false) => "  [ ] ",
        (true, false, true) => "  [x] ",
        (true, false, false) => "  [ ] ",
        (false, true, true) => "  > ",
        (false, true, false) => "  > ",
        (false, false, true) => "  * ",
        (false, false, false) => "    ",
    }
}

fn draw_status(frame: &mut ratatui::Frame, state: &AskState, area: ratatui::layout::Rect) {
    let key = Style::default().fg(Color::Rgb(177, 185, 249));
    let spans = if state.mode == InputMode::TextInput {
        vec![
            Span::styled(" Type your answer ", Style::default().fg(Color::White)),
            Span::styled("Enter ", key),
            Span::raw("Next  "),
            Span::styled("Esc ", key),
            Span::raw("Cancel"),
        ]
    } else if state.on_submit_tab() {
        vec![
            Span::styled(" Enter ", key),
            Span::raw("Submit  "),
            Span::styled("Tab ", key),
            Span::raw("Back  "),
            Span::styled("Esc ", key),
            Span::raw("Cancel"),
        ]
    } else {
        vec![
            Span::styled(" ↑↓ ", key),
            Span::raw("Select  "),
            Span::styled("Enter ", key),
            Span::raw("Next  "),
            Span::styled("Tab ", key),
            Span::raw("Next  "),
            Span::styled("Space ", key),
            Span::raw("Toggle  "),
            Span::styled("Esc ", key),
            Span::raw("Cancel"),
        ]
    };

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
