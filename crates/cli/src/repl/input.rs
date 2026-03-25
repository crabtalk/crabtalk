//! Bordered multi-line input box with dropdown completion.
//!
//! Uses ratatui `Viewport::Inline` for the input box — correct width
//! calculations, Unicode handling, and border rendering out of the box.

use crate::repl::command::collect_candidates;
use crate::tui;
use crossterm::{
    cursor, event,
    style::{self, Attribute, Color, SetAttribute, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};
use ratatui::{
    layout::Alignment,
    style::{Color as RColor, Style as RStyle},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use std::io::Write;

const MAX_DROPDOWN_ROWS: usize = 5;

/// Result of reading input.
pub enum InputResult {
    /// User submitted content (may be multi-line).
    Line(String),
    /// User pressed Ctrl+C.
    Interrupt,
    /// User pressed Ctrl+D on empty input.
    Eof,
    /// User pressed Ctrl+L — clear the screen.
    ClearScreen,
}

/// Command history backed by a Vec.
pub struct History {
    entries: Vec<String>,
    cursor: usize,
    stash: String,
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

// ── Main read function ────────────────────────────────────────────

/// Read multi-line input with a ratatui-rendered bordered box.
pub fn read_line(agent: &str, history: &mut History, title: &str) -> InputResult {
    if terminal::enable_raw_mode().is_err() {
        return InputResult::Eof;
    }

    let mut buf = InputBuffer::new();
    let result = input_loop(agent, title, &mut buf, history);

    let _ = terminal::disable_raw_mode();
    result
}

fn input_loop(
    agent: &str,
    title: &str,
    buf: &mut InputBuffer,
    history: &mut History,
) -> InputResult {
    let mut stdout = std::io::stdout();
    // Track the row where the box starts for absolute erase.
    let mut box_start_row: Option<u16> = None;
    let mut last_height: u16 = 0;

    loop {
        draw_input_box(
            &mut stdout,
            agent,
            title,
            buf,
            &mut box_start_row,
            &mut last_height,
        );

        let Ok(ev) = event::read() else { continue };

        // On terminal resize, clear entire screen and redraw fresh.
        if matches!(ev, event::Event::Resize(..)) {
            let _ = crossterm::execute!(stdout, Clear(ClearType::All), cursor::MoveTo(0, 0),);
            box_start_row.take();
            last_height = 0;
            continue;
        }

        let event::Event::Key(key) = ev else { continue };

        // Helper: erase the box before exiting.
        let erase = |stdout: &mut std::io::Stdout, start: Option<u16>| {
            if let Some(row) = start {
                let _ = crossterm::execute!(
                    stdout,
                    cursor::MoveTo(0, row),
                    Clear(ClearType::FromCursorDown),
                );
            }
        };

        // Ctrl+C
        if key.modifiers.contains(event::KeyModifiers::CONTROL)
            && key.code == event::KeyCode::Char('c')
        {
            erase(&mut stdout, box_start_row);
            return InputResult::Interrupt;
        }
        // Ctrl+D on empty
        if key.modifiers.contains(event::KeyModifiers::CONTROL)
            && key.code == event::KeyCode::Char('d')
            && buf.is_empty()
        {
            erase(&mut stdout, box_start_row);
            return InputResult::Eof;
        }
        // Ctrl+L
        if key.modifiers.contains(event::KeyModifiers::CONTROL)
            && key.code == event::KeyCode::Char('l')
        {
            erase(&mut stdout, box_start_row);
            return InputResult::ClearScreen;
        }

        match key.code {
            event::KeyCode::Enter => {
                if key.modifiers.contains(event::KeyModifiers::SHIFT) {
                    buf.insert_newline();
                } else {
                    let content = buf.content();
                    erase(&mut stdout, box_start_row);
                    return InputResult::Line(content);
                }
            }
            event::KeyCode::Up => {
                if buf.is_multiline() && buf.cursor.0 > 0 {
                    buf.move_up();
                } else if let Some(entry) = history.prev(&buf.content()) {
                    *buf = InputBuffer::from_str(entry);
                }
            }
            event::KeyCode::Down => {
                if buf.is_multiline() && buf.cursor.0 + 1 < buf.lines.len() {
                    buf.move_down();
                } else if let Some(entry) = history.next() {
                    *buf = InputBuffer::from_str(entry);
                }
            }
            event::KeyCode::Tab => {
                if buf.first_line().starts_with('/') {
                    let result = run_dropdown(
                        buf,
                        box_start_row.unwrap_or(0),
                        last_height,
                        &mut stdout,
                        agent,
                        title,
                    );
                    if let Some(completed) = result {
                        *buf = InputBuffer::from_str(&format!("{completed} "));
                    }
                    // Clear from the OLD box position down to remove both the
                    // old box and any dropdown remnants.
                    if let Some(row) = box_start_row {
                        let _ = crossterm::execute!(
                            stdout,
                            cursor::MoveTo(0, row),
                            Clear(ClearType::FromCursorDown),
                        );
                    }
                    box_start_row.take();
                    last_height = 0;
                }
            }
            event::KeyCode::Char('/') if buf.is_empty() => {
                buf.handle_key(event::KeyCode::Char('/'));
                draw_input_box(
                    &mut stdout,
                    agent,
                    title,
                    buf,
                    &mut box_start_row,
                    &mut last_height,
                );
                let saved_row = box_start_row;
                let result = run_dropdown(
                    buf,
                    box_start_row.unwrap_or(0),
                    last_height,
                    &mut stdout,
                    agent,
                    title,
                );
                if let Some(completed) = result {
                    *buf = InputBuffer::from_str(&format!("{completed} "));
                }
                if let Some(row) = saved_row {
                    let _ = crossterm::execute!(
                        stdout,
                        cursor::MoveTo(0, row),
                        Clear(ClearType::FromCursorDown),
                    );
                }
                box_start_row.take();
                last_height = 0;
            }
            code => {
                let old_len = buf.content().len();
                buf.handle_key(code);
                if buf.content().len() != old_len {
                    history.reset_cursor();
                }
            }
        }
    }
}

// ── Ratatui rendering ─────────────────────────────────────────────

fn draw_input_box(
    stdout: &mut std::io::Stdout,
    agent: &str,
    title: &str,
    buf: &InputBuffer,
    box_start_row: &mut Option<u16>,
    last_height: &mut u16,
) {
    use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};

    let (cols, _) = terminal::size().unwrap_or((80, 24));
    let height = buf.lines.len() as u16 + 2; // lines + borders

    // Erase previous box: move to its start and clear everything below.
    if let Some(start_row) = *box_start_row {
        let _ = crossterm::execute!(
            stdout,
            cursor::MoveTo(0, start_row),
            Clear(ClearType::FromCursorDown),
        );
    }

    // Build the block.
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

    // Build input text.
    let lines: Vec<Line> = buf
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

    // Render to a buffer and print line by line.
    let area = Rect::new(0, 0, cols, height);
    let mut render_buf = Buffer::empty(area);
    paragraph.render(area, &mut render_buf);

    let _ = crossterm::execute!(stdout, cursor::MoveToColumn(0));
    // Record the row where the box starts.
    let (_, start_row) = crossterm::cursor::position().unwrap_or((0, 0));
    *box_start_row = Some(start_row);

    // Render the buffer to a string with ANSI codes, one row at a time.
    // This is MUCH faster than per-cell execute! calls.
    for y in 0..height {
        if y > 0 {
            write!(stdout, "\r\n").ok();
        }
        let mut last_fg = RColor::Reset;
        let mut last_bg = RColor::Reset;
        for x in 0..cols {
            let cell = &render_buf[(x, y)];
            if cell.fg != last_fg {
                if cell.fg == RColor::Reset {
                    write!(stdout, "\x1b[39m").ok(); // default fg
                } else {
                    let _ =
                        crossterm::queue!(stdout, SetForegroundColor(to_crossterm_color(cell.fg)));
                }
                last_fg = cell.fg;
            }
            if cell.bg != last_bg {
                if cell.bg == RColor::Reset {
                    write!(stdout, "\x1b[49m").ok(); // default bg
                } else {
                    let _ = crossterm::queue!(
                        stdout,
                        crossterm::style::SetBackgroundColor(to_crossterm_color(cell.bg))
                    );
                }
                last_bg = cell.bg;
            }
            write!(stdout, "{}", cell.symbol()).ok();
        }
        // Reset at end of each row.
        write!(stdout, "\x1b[0m").ok();
    }

    *last_height = height;

    // Position cursor.
    let (cur_line, cur_col) = buf.cursor;
    let prefix_w: u16 = if cur_line == 0 { 2 } else { 3 };
    let col = 1 + prefix_w + cur_col as u16;
    let rows_up = height - 1 - (1 + cur_line as u16);
    if rows_up > 0 {
        let _ = crossterm::queue!(stdout, cursor::MoveUp(rows_up));
    }
    let _ = crossterm::queue!(stdout, cursor::MoveToColumn(col));
    let _ = stdout.flush();
}

fn to_crossterm_color(c: RColor) -> Color {
    match c {
        RColor::Rgb(r, g, b) => Color::Rgb { r, g, b },
        RColor::Indexed(i) => Color::AnsiValue(i),
        RColor::White => Color::White,
        RColor::DarkGray => Color::DarkGrey,
        _ => Color::Reset,
    }
}

// ── Dropdown ──────────────────────────────────────────────────────

fn run_dropdown(
    buf: &mut InputBuffer,
    box_start_row: u16,
    box_height: u16,
    stdout: &mut std::io::Stdout,
    agent: &str,
    title: &str,
) -> Option<String> {
    let line = buf.first_line().to_string();
    let candidates = collect_candidates(&line, line.len());
    match candidates.len() {
        0 => None,
        1 => Some(candidates.into_iter().next().unwrap()),
        _ => show_dropdown(
            &candidates,
            buf,
            box_start_row,
            box_height,
            stdout,
            agent,
            title,
        ),
    }
}

fn show_dropdown(
    candidates: &[String],
    input_buf: &mut InputBuffer,
    box_start_row: u16,
    box_height: u16,
    stdout: &mut std::io::Stdout,
    agent: &str,
    title: &str,
) -> Option<String> {
    let max_visible = MAX_DROPDOWN_ROWS.min(candidates.len());
    let mut selected: usize = 0;
    let mut scroll: usize = 0;
    let mut filter = String::new();
    let mut filtered: Vec<&str> = candidates.iter().map(|s| s.as_str()).collect();
    let mut max_drawn: u16 = 0;

    // Dropdown starts right below the input box's bottom border.
    let dropdown_start = box_start_row + box_height;

    // Pre-scroll to ensure room for dropdown.
    let dropdown_rows = max_visible as u16 + 1;
    let (_, term_rows) = terminal::size().unwrap_or((80, 24));
    let needed_bottom = dropdown_start + dropdown_rows;
    if needed_bottom > term_rows {
        let scroll_by = needed_bottom - term_rows;
        for _ in 0..scroll_by {
            let _ = crossterm::execute!(stdout, style::Print("\n"));
        }
        // Box moved up by scroll_by — but we don't need to track that
        // since the dropdown position is relative to the scrolled position.
    }

    loop {
        // Clear old dropdown lines.
        for i in 0..max_drawn {
            let _ = crossterm::execute!(
                stdout,
                cursor::MoveTo(0, dropdown_start + i),
                Clear(ClearType::CurrentLine),
            );
        }

        if filtered.is_empty() {
            return None;
        }

        let mut drawn: u16 = 0;
        if selected >= filtered.len() {
            selected = filtered.len() - 1;
        }
        let vis = max_visible.min(filtered.len());
        if selected < scroll {
            scroll = selected;
        } else if selected >= scroll + vis {
            scroll = selected + 1 - vis;
        }

        for (i, &item) in filtered[scroll..scroll + vis].iter().enumerate() {
            let row = dropdown_start + i as u16;
            let _ = crossterm::execute!(stdout, cursor::MoveTo(0, row));
            if scroll + i == selected {
                let _ = crossterm::execute!(
                    stdout,
                    SetForegroundColor(Color::AnsiValue(173)),
                    SetAttribute(Attribute::Bold),
                    style::Print(format!("  > {item}")),
                    SetAttribute(Attribute::Reset),
                    style::ResetColor,
                );
            } else {
                let _ = crossterm::execute!(
                    stdout,
                    SetForegroundColor(Color::DarkGrey),
                    style::Print(format!("    {item}")),
                    style::ResetColor,
                );
            }
            drawn += 1;
        }

        if filtered.len() > vis {
            let row = dropdown_start + drawn;
            let _ = crossterm::execute!(
                stdout,
                cursor::MoveTo(0, row),
                SetForegroundColor(Color::DarkGrey),
                style::Print(format!("    ({}/{})", vis, filtered.len())),
                style::ResetColor,
            );
            drawn += 1;
        }

        if drawn > max_drawn {
            max_drawn = drawn;
        }

        // Park cursor at dropdown area.
        let _ = crossterm::execute!(stdout, cursor::MoveTo(0, dropdown_start));
        let _ = stdout.flush();

        let Ok(event::Event::Key(key)) = event::read() else {
            continue;
        };

        // Inline cleanup helper — clears dropdown lines.
        macro_rules! cleanup {
            () => {
                for i in 0..drawn.max(max_drawn) {
                    let _ = crossterm::execute!(
                        stdout,
                        cursor::MoveTo(0, dropdown_start + i),
                        Clear(ClearType::CurrentLine),
                    );
                }
            };
        }

        match key.code {
            event::KeyCode::Up => selected = selected.saturating_sub(1),
            event::KeyCode::Down => {
                if !filtered.is_empty() {
                    selected = (selected + 1).min(filtered.len() - 1);
                }
            }
            event::KeyCode::Enter | event::KeyCode::Tab => {
                let result = filtered.get(selected).map(|s| s.to_string());
                cleanup!();
                return result;
            }
            event::KeyCode::Esc => {
                cleanup!();
                return None;
            }
            event::KeyCode::Char(' ') => {
                cleanup!();
                return Some(format!("/{filter}"));
            }
            event::KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                cleanup!();
                return None;
            }
            event::KeyCode::Backspace => {
                filter.pop();
                input_buf.handle_key(event::KeyCode::Backspace);
                if filter.is_empty() {
                    cleanup!();
                    return None;
                }
                filtered = candidates
                    .iter()
                    .filter(|c| c.contains(filter.as_str()))
                    .map(|s| s.as_str())
                    .collect();
                selected = 0;
                scroll = 0;
                // Redraw the input box to show current text.
                let mut bsr = Some(box_start_row);
                let mut bh = box_height;
                draw_input_box(stdout, agent, title, input_buf, &mut bsr, &mut bh);
            }
            event::KeyCode::Char(ch) => {
                filter.push(ch);
                input_buf.handle_key(event::KeyCode::Char(ch));
                filtered = candidates
                    .iter()
                    .filter(|c| c.contains(filter.as_str()))
                    .map(|s| s.as_str())
                    .collect();
                selected = 0;
                scroll = 0;
                // Redraw the input box to show current text.
                let mut bsr = Some(box_start_row);
                let mut bh = box_height;
                draw_input_box(stdout, agent, title, input_buf, &mut bsr, &mut bh);
            }
            _ => {}
        }
    }
}
