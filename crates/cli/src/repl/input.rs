//! Bordered multi-line input box with dropdown completion.
//!
//! Renders a box with top/bottom borders around the input area.
//! Supports multi-line input (Shift+Enter), history, and slash completion.

use crate::repl::command::collect_candidates;
use crate::tui;
use crossterm::{
    cursor, event,
    style::{self, Attribute, Color, SetAttribute, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};
use std::io::Write;

const MAX_DROPDOWN_ROWS: usize = 5;
const BORDER_COLOR: Color = Color::Rgb {
    r: 136,
    g: 136,
    b: 136,
};
const PROMPT_COLOR: Color = Color::AnsiValue(173);

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
    /// (line_index, char_position_in_line)
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

    /// Insert a newline at the cursor position (Shift+Enter).
    fn insert_newline(&mut self) {
        let (row, col) = self.cursor;
        let byte_pos = tui::char_to_byte(&self.lines[row], col);
        let rest = self.lines[row][byte_pos..].to_string();
        self.lines[row].truncate(byte_pos);
        self.lines.insert(row + 1, rest);
        self.cursor = (row + 1, 0);
    }

    /// Handle a text editing key on the current line.
    fn handle_key(&mut self, code: event::KeyCode) {
        let (row, col) = self.cursor;
        match code {
            event::KeyCode::Backspace => {
                if col > 0 {
                    tui::handle_text_input(code, &mut self.lines[row], &mut self.cursor.1);
                } else if row > 0 {
                    // Join with previous line.
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
            event::KeyCode::Home => {
                self.cursor.1 = 0;
            }
            event::KeyCode::End => {
                self.cursor.1 = self.lines[row].chars().count();
            }
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

/// Read multi-line input with bordered box.
pub fn read_line(prompt: &str, history: &mut History) -> InputResult {
    if terminal::enable_raw_mode().is_err() {
        return InputResult::Eof;
    }

    let prompt_width = console::measure_text_width(prompt);
    let mut buf = InputBuffer::new();
    let mut stdout = std::io::stdout();
    // Pre-scroll to ensure room for the box (top border + 1 line + bottom border = 3 rows).
    pre_scroll(&mut stdout, 3);
    let mut box_row = get_box_row(3);

    render_box(&mut stdout, prompt, &buf, prompt_width, box_row);

    let result = loop {
        let Ok(ev) = event::read() else { continue };
        let event::Event::Key(key) = ev else { continue };

        // Ctrl+C
        if key.modifiers.contains(event::KeyModifiers::CONTROL)
            && key.code == event::KeyCode::Char('c')
        {
            erase_box(&mut stdout, buf.lines.len() as u16, box_row);
            break InputResult::Interrupt;
        }
        // Ctrl+D on empty
        if key.modifiers.contains(event::KeyModifiers::CONTROL)
            && key.code == event::KeyCode::Char('d')
            && buf.is_empty()
        {
            erase_box(&mut stdout, buf.lines.len() as u16, box_row);
            break InputResult::Eof;
        }
        // Ctrl+L
        if key.modifiers.contains(event::KeyModifiers::CONTROL)
            && key.code == event::KeyCode::Char('l')
        {
            erase_box(&mut stdout, buf.lines.len() as u16, box_row);
            break InputResult::ClearScreen;
        }

        let old_line_count = buf.lines.len() as u16;

        match key.code {
            event::KeyCode::Enter => {
                if key.modifiers.contains(event::KeyModifiers::SHIFT) {
                    // Shift+Enter: new line.
                    buf.insert_newline();
                    // Re-scroll if needed for the extra line.
                    let new_height = buf.lines.len() as u16 + 2; // lines + borders
                    erase_box(&mut stdout, old_line_count, box_row);
                    pre_scroll(&mut stdout, new_height);
                    box_row = get_box_row(new_height);
                } else {
                    // Submit.
                    let content = buf.content();
                    erase_box(&mut stdout, old_line_count, box_row);
                    let _ = stdout.flush();
                    break InputResult::Line(content);
                }
            }
            event::KeyCode::Up => {
                if buf.is_multiline() && buf.cursor.0 > 0 {
                    buf.move_up();
                } else if let Some(entry) = history.prev(&buf.content()) {
                    erase_box(&mut stdout, old_line_count, box_row);
                    buf = InputBuffer::from_str(entry);
                    let new_height = buf.lines.len() as u16 + 2;
                    pre_scroll(&mut stdout, new_height);
                    box_row = get_box_row(new_height);
                }
            }
            event::KeyCode::Down => {
                if buf.is_multiline() && buf.cursor.0 + 1 < buf.lines.len() {
                    buf.move_down();
                } else if let Some(entry) = history.next() {
                    erase_box(&mut stdout, old_line_count, box_row);
                    buf = InputBuffer::from_str(entry);
                    let new_height = buf.lines.len() as u16 + 2;
                    pre_scroll(&mut stdout, new_height);
                    box_row = get_box_row(new_height);
                }
            }
            event::KeyCode::Tab => {
                if buf.first_line().starts_with('/')
                    && let Some(completed) =
                        run_dropdown(&mut stdout, prompt, &buf, prompt_width, box_row)
                {
                    erase_box(&mut stdout, old_line_count, box_row);
                    buf = InputBuffer::from_str(&format!("{completed} "));
                    pre_scroll(&mut stdout, 3);
                    box_row = get_box_row(3);
                }
            }
            event::KeyCode::Char('/') if buf.is_empty() => {
                buf.handle_key(event::KeyCode::Char('/'));
                render_box(&mut stdout, prompt, &buf, prompt_width, box_row);
                if let Some(completed) =
                    run_dropdown(&mut stdout, prompt, &buf, prompt_width, box_row)
                {
                    erase_box(&mut stdout, buf.lines.len() as u16, box_row);
                    buf = InputBuffer::from_str(&format!("{completed} "));
                    pre_scroll(&mut stdout, 3);
                    box_row = get_box_row(3);
                }
            }
            code => {
                let old_len = buf.content().len();
                buf.handle_key(code);
                if buf.content().len() != old_len {
                    history.reset_cursor();
                }
            }
        }

        render_box(&mut stdout, prompt, &buf, prompt_width, box_row);
    };

    let _ = terminal::disable_raw_mode();
    result
}

// ── Box rendering ─────────────────────────────────────────────────

fn render_box(
    stdout: &mut std::io::Stdout,
    prompt: &str,
    buf: &InputBuffer,
    _prompt_width: usize,
    box_row: u16,
) {
    let width = terminal::size().map(|(w, _)| w as usize).unwrap_or(80);

    // Top border: ┌─ prompt ──────────┐
    let _ = crossterm::execute!(
        stdout,
        cursor::MoveTo(0, box_row),
        Clear(ClearType::CurrentLine)
    );
    let prompt_label = prompt.trim();
    let border_fill = width.saturating_sub(4 + console::measure_text_width(prompt_label));
    let _ = crossterm::execute!(
        stdout,
        SetForegroundColor(BORDER_COLOR),
        style::Print("┌─ "),
        SetForegroundColor(PROMPT_COLOR),
        style::Print(prompt_label),
        SetForegroundColor(BORDER_COLOR),
        style::Print(format!(" {}", "─".repeat(border_fill))),
        style::ResetColor,
    );

    // Content lines.
    for (i, line) in buf.lines.iter().enumerate() {
        let row = box_row + 1 + i as u16;
        let _ = crossterm::execute!(
            stdout,
            cursor::MoveTo(0, row),
            Clear(ClearType::CurrentLine)
        );

        let prefix_width = if i == 0 { 4 } else { 5 };

        let _ = crossterm::execute!(
            stdout,
            SetForegroundColor(BORDER_COLOR),
            style::Print("│"),
            style::ResetColor,
        );

        if i == 0 {
            let _ = crossterm::execute!(
                stdout,
                SetForegroundColor(PROMPT_COLOR),
                style::Print(" > "),
                style::ResetColor,
            );
        } else {
            let _ = crossterm::execute!(
                stdout,
                SetForegroundColor(Color::DarkGrey),
                style::Print(" .. "),
                style::ResetColor,
            );
        }

        // Print line content with slash highlighting (command word only).
        if i == 0 && line.starts_with('/') {
            let (cmd, rest) = line.split_once(' ').unwrap_or((line, ""));
            let _ = crossterm::execute!(
                stdout,
                SetForegroundColor(Color::AnsiValue(240)),
                style::Print(cmd),
                style::ResetColor,
            );
            if !rest.is_empty() {
                let _ = crossterm::execute!(stdout, style::Print(format!(" {rest}")));
            }
        } else {
            let _ = crossterm::execute!(stdout, style::Print(line));
        }

        // Right border.
        let content_width: usize = line.chars().map(unicode_width).sum();
        let padding = width.saturating_sub(prefix_width + content_width + 1);
        let _ = crossterm::execute!(
            stdout,
            style::Print(" ".repeat(padding)),
            SetForegroundColor(BORDER_COLOR),
            style::Print("│"),
            style::ResetColor,
        );
    }

    // Bottom border.
    let bottom_row = box_row + 1 + buf.lines.len() as u16;
    let _ = crossterm::execute!(
        stdout,
        cursor::MoveTo(0, bottom_row),
        Clear(ClearType::CurrentLine),
        SetForegroundColor(BORDER_COLOR),
        style::Print(format!("└{}┘", "─".repeat(width.saturating_sub(2)))),
        style::ResetColor,
    );

    // Position cursor in the active line.
    let (cur_line, cur_col) = buf.cursor;
    let prefix_width: usize = if cur_line == 0 { 4 } else { 5 };
    let col = prefix_width
        + buf.lines[cur_line]
            .chars()
            .take(cur_col)
            .map(unicode_width)
            .sum::<usize>();
    let _ = crossterm::execute!(
        stdout,
        cursor::MoveTo(col as u16, box_row + 1 + cur_line as u16)
    );
    let _ = stdout.flush();
}

fn erase_box(stdout: &mut std::io::Stdout, line_count: u16, box_row: u16) {
    let total = line_count + 2; // top border + lines + bottom border
    for i in 0..total {
        let _ = crossterm::execute!(
            stdout,
            cursor::MoveTo(0, box_row + i),
            Clear(ClearType::CurrentLine),
        );
    }
    let _ = crossterm::execute!(stdout, cursor::MoveTo(0, box_row));
    let _ = stdout.flush();
}

fn pre_scroll(stdout: &mut std::io::Stdout, height: u16) {
    for _ in 0..height {
        let _ = crossterm::execute!(stdout, style::Print("\n"));
    }
}

fn get_box_row(height: u16) -> u16 {
    let (_, bottom) = crossterm::cursor::position().unwrap_or((0, height));
    bottom.saturating_sub(height)
}

// ── Dropdown (adapted for box layout) ─────────────────────────────

fn run_dropdown(
    stdout: &mut std::io::Stdout,
    prompt: &str,
    input_buf: &InputBuffer,
    prompt_width: usize,
    box_row: u16,
) -> Option<String> {
    let buf = input_buf.first_line();
    let candidates = collect_candidates(buf, buf.len());
    match candidates.len() {
        0 => None,
        1 => Some(candidates.into_iter().next().unwrap()),
        _ => show_dropdown(
            stdout,
            &candidates,
            box_row,
            prompt,
            input_buf,
            prompt_width,
        ),
    }
}

/// Interactive dropdown below the input box.
fn show_dropdown(
    stdout: &mut std::io::Stdout,
    candidates: &[String],
    orig_box_row: u16,
    prompt: &str,
    input_buf: &InputBuffer,
    prompt_width: usize,
) -> Option<String> {
    let max_visible = MAX_DROPDOWN_ROWS.min(candidates.len());
    let mut selected: usize = 0;
    let mut scroll: usize = 0;
    let mut filter = String::new();
    let mut filtered: Vec<&str> = candidates.iter().map(|s| s.as_str()).collect();
    let mut max_drawn: u16 = 0;

    // Erase old box, pre-scroll for box + dropdown, recalculate.
    let box_lines = input_buf.lines.len() as u16;
    erase_box(stdout, box_lines, orig_box_row);

    let box_height = box_lines + 2;
    let dropdown_rows = max_visible as u16 + 1;
    pre_scroll(stdout, box_height + dropdown_rows);
    let box_row = get_box_row(box_height + dropdown_rows);
    let dropdown_start = box_row + box_height;

    // Mutable copy for filter display.
    let mut line_buf = input_buf.first_line().to_string();
    let mut line_cursor = input_buf.cursor.1;

    loop {
        // Redraw box with current input.
        let display_buf = InputBuffer::from_str(&line_buf);
        render_box(stdout, prompt, &display_buf, prompt_width, box_row);

        // Clear old dropdown lines.
        for i in 0..max_drawn {
            let _ = crossterm::execute!(
                stdout,
                cursor::MoveTo(0, dropdown_start + i),
                Clear(ClearType::CurrentLine),
            );
        }

        if filtered.is_empty() {
            // No matches — clean up and exit.
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

        // Park cursor in the input line.
        let cursor_col = 4 + line_buf
            .chars()
            .take(line_cursor)
            .map(unicode_width)
            .sum::<usize>();
        let _ = crossterm::execute!(stdout, cursor::MoveTo(cursor_col as u16, box_row + 1));
        let _ = stdout.flush();

        let Ok(event::Event::Key(key)) = event::read() else {
            continue;
        };

        // Helper: clean up dropdown lines.
        let cleanup = |stdout: &mut std::io::Stdout| {
            for i in 0..drawn.max(max_drawn) {
                let _ = crossterm::execute!(
                    stdout,
                    cursor::MoveTo(0, dropdown_start + i),
                    Clear(ClearType::CurrentLine),
                );
            }
        };

        match key.code {
            event::KeyCode::Up => selected = selected.saturating_sub(1),
            event::KeyCode::Down => {
                if !filtered.is_empty() {
                    selected = (selected + 1).min(filtered.len() - 1);
                }
            }
            event::KeyCode::Enter | event::KeyCode::Tab => {
                let result = filtered.get(selected).map(|s| s.to_string());
                cleanup(stdout);
                return result;
            }
            event::KeyCode::Esc => {
                cleanup(stdout);
                return None;
            }
            event::KeyCode::Char(' ') => {
                cleanup(stdout);
                return Some(line_buf);
            }
            event::KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                cleanup(stdout);
                return None;
            }
            event::KeyCode::Backspace => {
                if filter.pop().is_some() && line_cursor > 1 {
                    tui::handle_text_input(
                        event::KeyCode::Backspace,
                        &mut line_buf,
                        &mut line_cursor,
                    );
                    filtered = candidates
                        .iter()
                        .filter(|c| c.contains(filter.as_str()))
                        .map(|s| s.as_str())
                        .collect();
                    selected = 0;
                    scroll = 0;
                }
                // If the entire input (including the initial `/`) is gone, exit.
                if line_buf.is_empty() || line_buf == "/" && filter.is_empty() {
                    cleanup(stdout);
                    return None;
                }
            }
            event::KeyCode::Char(ch) => {
                filter.push(ch);
                tui::handle_text_input(event::KeyCode::Char(ch), &mut line_buf, &mut line_cursor);
                filtered = candidates
                    .iter()
                    .filter(|c| c.contains(filter.as_str()))
                    .map(|s| s.as_str())
                    .collect();
                selected = 0;
                scroll = 0;
            }
            _ => {}
        }
    }
}

/// Approximate display width of a character.
fn unicode_width(c: char) -> usize {
    if c.is_ascii() { 1 } else { 2 }
}
