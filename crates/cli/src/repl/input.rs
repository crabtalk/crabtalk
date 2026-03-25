//! Multi-line input widget for the full-screen REPL.

use crate::repl::command::collect_candidates;
use crate::tui;
use crossterm::event;
use ratatui::{
    layout::Alignment,
    style::{Color as RColor, Modifier as RModifier, Style as RStyle},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

const MAX_DROPDOWN_VISIBLE: usize = 5;

/// Command history backed by a Vec.
pub struct History {
    entries: Vec<String>,
    cursor: usize,
    stash: String,
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl History {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            cursor: 0,
            stash: String::new(),
        }
    }

    pub fn load(&mut self, path: &std::path::Path) {
        if let Ok(content) = std::fs::read_to_string(path) {
            self.entries = content.lines().map(String::from).collect();
            self.cursor = self.entries.len();
        }
    }

    pub fn save(&self, path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, self.entries.join("\n"));
    }

    pub fn push(&mut self, line: &str) {
        if !line.is_empty() && self.entries.last().map(|s| s.as_str()) != Some(line) {
            self.entries.push(line.to_string());
        }
        self.cursor = self.entries.len();
    }

    fn prev(&mut self, current: &str) -> Option<&str> {
        if self.cursor == self.entries.len() {
            self.stash = current.to_string();
        }
        if self.cursor > 0 {
            self.cursor -= 1;
            Some(&self.entries[self.cursor])
        } else {
            None
        }
    }

    fn next(&mut self) -> Option<&str> {
        if self.cursor < self.entries.len() {
            self.cursor += 1;
            if self.cursor == self.entries.len() {
                Some(&self.stash)
            } else {
                Some(&self.entries[self.cursor])
            }
        } else {
            None
        }
    }

    fn reset_cursor(&mut self) {
        self.cursor = self.entries.len();
    }
}

// ── Multi-line buffer ─────────────────────────────────────────────

struct InputBuffer {
    lines: Vec<String>,
    cursor: (usize, usize),
}

impl InputBuffer {
    fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor: (0, 0),
        }
    }

    fn from_str(s: &str) -> Self {
        let lines: Vec<String> = if s.is_empty() {
            vec![String::new()]
        } else {
            s.lines().map(String::from).collect()
        };
        let last = lines.len() - 1;
        let col = lines[last].chars().count();
        Self {
            lines,
            cursor: (last, col),
        }
    }

    fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    fn is_multiline(&self) -> bool {
        self.lines.len() > 1
    }

    fn content(&self) -> String {
        self.lines.join("\n")
    }

    fn first_line(&self) -> &str {
        &self.lines[0]
    }

    fn insert_newline(&mut self) {
        let (row, col) = self.cursor;
        let byte_pos = tui::char_to_byte(&self.lines[row], col);
        let rest = self.lines[row][byte_pos..].to_string();
        self.lines[row].truncate(byte_pos);
        self.lines.insert(row + 1, rest);
        self.cursor = (row + 1, 0);
    }

    fn handle_key(&mut self, code: event::KeyCode) {
        let (row, col) = self.cursor;
        match code {
            event::KeyCode::Backspace => {
                if col > 0 {
                    tui::handle_text_input(code, &mut self.lines[row], &mut self.cursor.1);
                } else if row > 0 {
                    let current = self.lines.remove(row);
                    self.cursor.0 = row - 1;
                    self.cursor.1 = self.lines[row - 1].chars().count();
                    self.lines[row - 1].push_str(&current);
                }
            }
            event::KeyCode::Left => {
                if col > 0 {
                    self.cursor.1 -= 1;
                } else if row > 0 {
                    self.cursor.0 -= 1;
                    self.cursor.1 = self.lines[row - 1].chars().count();
                }
            }
            event::KeyCode::Right => {
                let line_len = self.lines[row].chars().count();
                if col < line_len {
                    self.cursor.1 += 1;
                } else if row + 1 < self.lines.len() {
                    self.cursor.0 += 1;
                    self.cursor.1 = 0;
                }
            }
            event::KeyCode::Home => self.cursor.1 = 0,
            event::KeyCode::End => self.cursor.1 = self.lines[row].chars().count(),
            _ => {
                tui::handle_text_input(code, &mut self.lines[row], &mut self.cursor.1);
            }
        }
    }

    fn move_up(&mut self) {
        if self.cursor.0 > 0 {
            self.cursor.0 -= 1;
            let line_len = self.lines[self.cursor.0].chars().count();
            self.cursor.1 = self.cursor.1.min(line_len);
        }
    }

    fn move_down(&mut self) {
        if self.cursor.0 + 1 < self.lines.len() {
            self.cursor.0 += 1;
            let line_len = self.lines[self.cursor.0].chars().count();
            self.cursor.1 = self.cursor.1.min(line_len);
        }
    }
}

// ── Dropdown ─────────────────────────────────────────────────────

struct DropdownState {
    candidates: Vec<String>,
    selected: usize,
    scroll: usize,
}

impl DropdownState {
    fn new(candidates: Vec<String>) -> Self {
        Self {
            candidates,
            selected: 0,
            scroll: 0,
        }
    }

    fn visible_range(&self) -> std::ops::Range<usize> {
        let vis = MAX_DROPDOWN_VISIBLE.min(self.candidates.len());
        if self.selected < self.scroll {
            // shouldn't happen, but be safe
            self.scroll..self.scroll + vis
        } else if self.selected >= self.scroll + vis {
            let new_scroll = self.selected + 1 - vis;
            new_scroll..new_scroll + vis
        } else {
            self.scroll..self.scroll + vis
        }
    }

    fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        }
    }

    fn move_down(&mut self) {
        if !self.candidates.is_empty() {
            self.selected = (self.selected + 1).min(self.candidates.len() - 1);
            let vis = MAX_DROPDOWN_VISIBLE.min(self.candidates.len());
            if self.selected >= self.scroll + vis {
                self.scroll = self.selected + 1 - vis;
            }
        }
    }

    fn current(&self) -> Option<&str> {
        self.candidates.get(self.selected).map(|s| s.as_str())
    }

    fn visible_height(&self) -> u16 {
        let h = MAX_DROPDOWN_VISIBLE.min(self.candidates.len());
        if self.candidates.len() > MAX_DROPDOWN_VISIBLE {
            h as u16 + 1 // +1 for count indicator
        } else {
            h as u16
        }
    }
}

// ── Public API ───────────────────────────────────────────────────

/// Action returned by [`InputState::handle_key`].
pub enum InputAction {
    /// User submitted content (Enter).
    Submit(String),
    /// User pressed Ctrl+C.
    Interrupt,
    /// User pressed Ctrl+D on empty input.
    Eof,
    /// Nothing to do (key consumed internally).
    Noop,
}

/// Input widget state for the full-screen REPL.
pub struct InputState {
    buf: InputBuffer,
    pub history: History,
    dropdown: Option<DropdownState>,
}

impl InputState {
    pub fn new(history: History) -> Self {
        Self {
            buf: InputBuffer::new(),
            history,
            dropdown: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Height of the input widget (content lines + 2 for borders).
    pub fn height(&self) -> u16 {
        self.buf.lines.len() as u16 + 2
    }

    fn open_dropdown(&mut self) {
        let line = self.buf.first_line().to_string();
        let candidates = collect_candidates(&line, line.len());
        if !candidates.is_empty() {
            self.dropdown = Some(DropdownState::new(candidates));
        }
    }

    fn close_dropdown(&mut self) {
        self.dropdown = None;
    }

    /// Process a key event.
    pub fn handle_key(&mut self, key: event::KeyEvent) -> InputAction {
        // Ctrl+C
        if key.modifiers.contains(event::KeyModifiers::CONTROL)
            && key.code == event::KeyCode::Char('c')
        {
            self.close_dropdown();
            return InputAction::Interrupt;
        }
        // Ctrl+D on empty
        if key.modifiers.contains(event::KeyModifiers::CONTROL)
            && key.code == event::KeyCode::Char('d')
            && self.buf.is_empty()
        {
            return InputAction::Eof;
        }

        // Dropdown active — intercept keys.
        if self.dropdown.is_some() {
            return self.handle_dropdown_key(key);
        }

        match key.code {
            event::KeyCode::Enter => {
                if key.modifiers.contains(event::KeyModifiers::SHIFT) {
                    self.buf.insert_newline();
                } else {
                    let content = self.buf.content();
                    self.history.push(&content);
                    self.buf = InputBuffer::new();
                    return InputAction::Submit(content);
                }
            }
            event::KeyCode::Up => {
                if self.buf.is_multiline() && self.buf.cursor.0 > 0 {
                    self.buf.move_up();
                } else if let Some(entry) = self.history.prev(&self.buf.content()) {
                    self.buf = InputBuffer::from_str(entry);
                }
            }
            event::KeyCode::Down => {
                if self.buf.is_multiline() && self.buf.cursor.0 + 1 < self.buf.lines.len() {
                    self.buf.move_down();
                } else if let Some(entry) = self.history.next() {
                    self.buf = InputBuffer::from_str(entry);
                }
            }
            event::KeyCode::Tab => {
                if self.buf.first_line().starts_with('/') {
                    self.open_dropdown();
                }
            }
            event::KeyCode::Char('/') if self.buf.is_empty() => {
                self.buf.handle_key(event::KeyCode::Char('/'));
                self.open_dropdown();
            }
            code => {
                let old_len = self.buf.content().len();
                self.buf.handle_key(code);
                if self.buf.content().len() != old_len {
                    self.history.reset_cursor();
                }
            }
        }
        InputAction::Noop
    }

    fn handle_dropdown_key(&mut self, key: event::KeyEvent) -> InputAction {
        match key.code {
            event::KeyCode::Up => {
                if let Some(dd) = &mut self.dropdown {
                    dd.move_up();
                }
            }
            event::KeyCode::Down => {
                if let Some(dd) = &mut self.dropdown {
                    dd.move_down();
                }
            }
            event::KeyCode::Enter | event::KeyCode::Tab => {
                if let Some(dd) = &self.dropdown
                    && let Some(selected) = dd.current()
                {
                    self.buf = InputBuffer::from_str(&format!("{selected} "));
                }
                self.close_dropdown();
            }
            event::KeyCode::Esc => {
                self.close_dropdown();
            }
            event::KeyCode::Char(' ') => {
                // Accept current prefix as typed.
                self.close_dropdown();
            }
            event::KeyCode::Backspace => {
                self.buf.handle_key(event::KeyCode::Backspace);
                if self.buf.is_empty() || !self.buf.first_line().starts_with('/') {
                    self.close_dropdown();
                } else {
                    // Re-filter candidates.
                    let line = self.buf.first_line().to_string();
                    let candidates = collect_candidates(&line, line.len());
                    if candidates.is_empty() {
                        self.close_dropdown();
                    } else if let Some(dd) = &mut self.dropdown {
                        dd.candidates = candidates;
                        dd.selected = dd.selected.min(dd.candidates.len().saturating_sub(1));
                        dd.scroll = dd.scroll.min(dd.candidates.len().saturating_sub(1));
                    }
                }
            }
            event::KeyCode::Char(ch) => {
                self.buf.handle_key(event::KeyCode::Char(ch));
                // Re-filter candidates.
                let line = self.buf.first_line().to_string();
                let candidates = collect_candidates(&line, line.len());
                if candidates.is_empty() {
                    self.close_dropdown();
                } else if let Some(dd) = &mut self.dropdown {
                    dd.candidates = candidates;
                    dd.selected = dd.selected.min(dd.candidates.len().saturating_sub(1));
                    dd.scroll = dd.scroll.min(dd.candidates.len().saturating_sub(1));
                }
            }
            _ => {}
        }
        InputAction::Noop
    }

    /// Render the input box into the given area.
    pub fn render(
        &self,
        frame: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        agent: &str,
        title: &str,
    ) {
        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_style(RStyle::default().fg(RColor::Rgb(136, 136, 136)))
            .title_top(
                Line::from(format!(" {agent} > "))
                    .style(RStyle::default().fg(RColor::Rgb(215, 119, 87))),
            );

        if !title.is_empty() {
            block = block.title_top(
                Line::from(vec![
                    Span::styled(
                        format!(" {title} "),
                        RStyle::default()
                            .fg(RColor::White)
                            .bg(RColor::Rgb(60, 60, 60)),
                    ),
                    Span::styled("─", RStyle::default().fg(RColor::Rgb(136, 136, 136))),
                ])
                .alignment(Alignment::Right),
            );
        }

        let lines: Vec<Line> = self
            .buf
            .lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let prefix = if i == 0 { "> " } else { ".. " };
                let prefix_style = if i == 0 {
                    RStyle::default().fg(RColor::Rgb(215, 119, 87))
                } else {
                    RStyle::default().fg(RColor::DarkGray)
                };

                if i == 0 && line.starts_with('/') {
                    let (cmd, rest) = line.split_once(' ').unwrap_or((line, ""));
                    let mut spans = vec![
                        Span::styled(prefix, prefix_style),
                        Span::styled(
                            cmd.to_string(),
                            RStyle::default().fg(RColor::Rgb(160, 160, 160)),
                        ),
                    ];
                    if !rest.is_empty() {
                        spans.push(Span::raw(format!(" {rest}")));
                    }
                    Line::from(spans)
                } else {
                    Line::from(vec![
                        Span::styled(prefix, prefix_style),
                        Span::raw(line.as_str()),
                    ])
                }
            })
            .collect();

        let paragraph = Paragraph::new(lines).block(block);
        frame.render_widget(paragraph, area);

        // Position cursor inside the input box.
        let (cur_line, cur_col) = self.buf.cursor;
        let prefix_w: u16 = if cur_line == 0 { 2 } else { 3 };
        let x = area.x + 1 + prefix_w + cur_col as u16;
        let y = area.y + 1 + cur_line as u16;
        frame.set_cursor_position((x, y));

        // Dropdown overlay (rendered above the input box).
        if let Some(dd) = &self.dropdown {
            let dd_height = dd.visible_height();
            if dd_height > 0 && area.y >= dd_height {
                let dd_area = ratatui::layout::Rect::new(
                    area.x + 1,
                    area.y - dd_height,
                    area.width.saturating_sub(2).min(40),
                    dd_height,
                );
                frame.render_widget(Clear, dd_area);
                let range = dd.visible_range();
                let mut dd_lines = Vec::new();
                for (i, item) in dd.candidates[range.clone()].iter().enumerate() {
                    let idx = range.start + i;
                    if idx == dd.selected {
                        dd_lines.push(Line::from(Span::styled(
                            format!("  > {item}"),
                            RStyle::default()
                                .fg(RColor::Rgb(215, 119, 87))
                                .add_modifier(RModifier::BOLD),
                        )));
                    } else {
                        dd_lines.push(Line::from(Span::styled(
                            format!("    {item}"),
                            RStyle::default().fg(RColor::DarkGray),
                        )));
                    }
                }
                if dd.candidates.len() > MAX_DROPDOWN_VISIBLE {
                    dd_lines.push(Line::from(Span::styled(
                        format!(
                            "    ({}/{})",
                            MAX_DROPDOWN_VISIBLE.min(dd.candidates.len()),
                            dd.candidates.len()
                        ),
                        RStyle::default().fg(RColor::DarkGray),
                    )));
                }
                frame.render_widget(Paragraph::new(dd_lines), dd_area);
            }
        }
    }
}
