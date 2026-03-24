//! Custom crossterm-based input line with dropdown completion.
//!
//! Replaces rustyline — we own raw mode and cursor tracking, so the
//! dropdown can draw below the prompt without corrupting anything.

use crate::repl::command::collect_candidates;
use crate::tui;
use crossterm::{
    cursor, event,
    style::{self, Attribute, Color, SetAttribute, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};
use std::io::Write;

const MAX_DROPDOWN_ROWS: usize = 5;

/// Result of reading a line of input.
pub enum InputResult {
    /// User submitted a line.
    Line(String),
    /// User pressed Ctrl+C.
    Interrupt,
    /// User pressed Ctrl+D on an empty line.
    Eof,
}

/// Command history backed by a Vec.
pub struct History {
    entries: Vec<String>,
    cursor: usize,
    /// Stash the in-progress line when navigating history.
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

    /// Load history from a file (one entry per line).
    pub fn load(&mut self, path: &std::path::Path) {
        if let Ok(content) = std::fs::read_to_string(path) {
            self.entries = content.lines().map(String::from).collect();
            self.cursor = self.entries.len();
        }
    }

    /// Save history to a file.
    pub fn save(&self, path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, self.entries.join("\n"));
    }

    /// Add an entry (deduplicates consecutive).
    pub fn push(&mut self, line: &str) {
        if !line.is_empty() && self.entries.last().map(|s| s.as_str()) != Some(line) {
            self.entries.push(line.to_string());
        }
        self.cursor = self.entries.len();
    }

    /// Navigate backward. Returns the entry to display.
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

    /// Navigate forward. Returns the entry or the stash.
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

    /// Reset cursor to end (after editing).
    fn reset_cursor(&mut self) {
        self.cursor = self.entries.len();
    }
}

/// Read a line of input with the given prompt.
///
/// Handles editing, history, slash highlighting, and dropdown completion.
/// Caller must NOT be in raw mode — this function manages it.
pub fn read_line(prompt: &str, history: &mut History) -> InputResult {
    // Ensure the input area isn't squeezed at the very bottom of the
    // terminal. If the cursor is within 2 rows of the bottom, scroll up.
    if let Ok((_, rows)) = terminal::size()
        && let Ok((_, cur_row)) = crossterm::cursor::position()
        && cur_row + 2 >= rows
    {
        let pad = 3u16.min(rows.saturating_sub(1));
        for _ in 0..pad {
            println!();
        }
        let _ = std::io::stdout().flush();
    }

    if terminal::enable_raw_mode().is_err() {
        return InputResult::Eof;
    }

    let prompt_width = console::measure_text_width(prompt);
    let mut buf = String::new();
    let mut cursor_pos: usize = 0;
    let mut stdout = std::io::stdout();

    // Initial render.
    render_line(&mut stdout, prompt, &buf, cursor_pos, prompt_width);

    let result = loop {
        let Ok(ev) = event::read() else { continue };
        let event::Event::Key(key) = ev else { continue };

        // Ctrl+C
        if key.modifiers.contains(event::KeyModifiers::CONTROL)
            && key.code == event::KeyCode::Char('c')
        {
            break InputResult::Interrupt;
        }
        // Ctrl+D on empty line
        if key.modifiers.contains(event::KeyModifiers::CONTROL)
            && key.code == event::KeyCode::Char('d')
            && buf.is_empty()
        {
            break InputResult::Eof;
        }

        match key.code {
            event::KeyCode::Enter => {
                // Move past the input line.
                let _ = crossterm::execute!(stdout, style::Print("\n"));
                break InputResult::Line(buf);
            }
            event::KeyCode::Up => {
                if let Some(entry) = history.prev(&buf) {
                    buf = entry.to_string();
                    cursor_pos = buf.chars().count();
                }
            }
            event::KeyCode::Down => {
                if let Some(entry) = history.next() {
                    buf = entry.to_string();
                    cursor_pos = buf.chars().count();
                }
            }
            event::KeyCode::Tab => {
                if buf.starts_with('/')
                    && let Some(completed) =
                        run_dropdown(&mut stdout, prompt, &buf, cursor_pos, prompt_width)
                {
                    buf = format!("{completed} ");
                    cursor_pos = buf.chars().count();
                }
            }
            event::KeyCode::Char('/') if buf.is_empty() => {
                // Type `/` and immediately open dropdown.
                buf.push('/');
                cursor_pos = 1;
                render_line(&mut stdout, prompt, &buf, cursor_pos, prompt_width);
                if let Some(completed) =
                    run_dropdown(&mut stdout, prompt, &buf, cursor_pos, prompt_width)
                {
                    buf = format!("{completed} ");
                    cursor_pos = buf.chars().count();
                }
            }
            code => {
                let old_len = buf.len();
                tui::handle_text_input(code, &mut buf, &mut cursor_pos);
                if buf.len() != old_len {
                    history.reset_cursor();
                }
            }
        }

        render_line(&mut stdout, prompt, &buf, cursor_pos, prompt_width);
    };

    let _ = terminal::disable_raw_mode();
    result
}

/// Render the prompt + buffer on the current line, positioning the cursor.
fn render_line(
    stdout: &mut std::io::Stdout,
    prompt: &str,
    buf: &str,
    cursor_pos: usize,
    prompt_width: usize,
) {
    let _ = crossterm::execute!(
        stdout,
        cursor::MoveToColumn(0),
        Clear(ClearType::CurrentLine)
    );

    // Print prompt.
    let _ = crossterm::execute!(stdout, style::Print(prompt));

    // Print buffer with slash highlighting.
    if buf.starts_with('/') {
        let _ = crossterm::execute!(
            stdout,
            SetForegroundColor(Color::AnsiValue(240)),
            style::Print(buf),
            style::ResetColor,
        );
    } else {
        let _ = crossterm::execute!(stdout, style::Print(buf));
    }

    // Position cursor.
    let col = prompt_width
        + buf
            .chars()
            .take(cursor_pos)
            .map(unicode_width)
            .sum::<usize>();
    let _ = crossterm::execute!(stdout, cursor::MoveToColumn(col as u16));
    let _ = stdout.flush();
}

/// Run the dropdown completion flow. Returns the selected candidate or None.
fn run_dropdown(
    stdout: &mut std::io::Stdout,
    prompt: &str,
    buf: &str,
    cursor_pos: usize,
    prompt_width: usize,
) -> Option<String> {
    let candidates = collect_candidates(buf, buf.len());
    match candidates.len() {
        0 => None,
        1 => Some(candidates.into_iter().next().unwrap()),
        _ => show_dropdown(stdout, prompt, buf, cursor_pos, prompt_width, &candidates),
    }
}

/// Interactive dropdown below the input line.
fn show_dropdown(
    stdout: &mut std::io::Stdout,
    prompt: &str,
    _buf: &str,
    _cursor_pos: usize,
    prompt_width: usize,
    candidates: &[String],
) -> Option<String> {
    let max_visible = MAX_DROPDOWN_ROWS.min(candidates.len());
    let mut selected: usize = 0;
    let mut scroll: usize = 0;
    let mut filter = String::new();
    let mut filtered: Vec<&str> = candidates.iter().map(|s| s.as_str()).collect();
    let mut max_drawn: u16 = 0;

    // Pre-scroll: ensure room below for dropdown items.
    let total_rows = max_visible as u16 + 1;
    for _ in 0..total_rows {
        let _ = crossterm::execute!(stdout, style::Print("\n"));
    }
    // Calculate input row after potential scroll.
    let (_, bottom_row) = crossterm::cursor::position().unwrap_or((0, total_rows));
    let input_row = bottom_row.saturating_sub(total_rows);

    // Internal buffer for the input line (starts as a copy of `buf`).
    // The dropdown updates this as the user filters.
    let mut line_buf = _buf.to_string();
    let mut line_cursor = _cursor_pos;

    loop {
        // Redraw the input line (in case filter chars were typed).
        let _ = crossterm::execute!(stdout, cursor::MoveTo(0, input_row));
        let _ = crossterm::execute!(stdout, Clear(ClearType::CurrentLine));
        let _ = crossterm::execute!(stdout, style::Print(prompt));
        if line_buf.starts_with('/') {
            let _ = crossterm::execute!(
                stdout,
                SetForegroundColor(Color::AnsiValue(240)),
                style::Print(&line_buf),
                style::ResetColor,
            );
        } else {
            let _ = crossterm::execute!(stdout, style::Print(&line_buf));
        }

        // Clear old dropdown lines.
        for i in 0..max_drawn {
            let _ = crossterm::execute!(
                stdout,
                cursor::MoveTo(0, input_row + 1 + i),
                Clear(ClearType::CurrentLine),
            );
        }

        let mut drawn: u16 = 0;
        if filtered.is_empty() {
            // No matches — auto-exit. Return None so caller keeps current buf.
            erase_below(stdout, max_drawn, input_row);
            // Re-render and position cursor on input line.
            let col = prompt_width
                + line_buf
                    .chars()
                    .take(line_cursor)
                    .map(unicode_width)
                    .sum::<usize>();
            let _ = crossterm::execute!(stdout, cursor::MoveTo(col as u16, input_row));
            let _ = stdout.flush();
            return None;
        }

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
            let row = input_row + 1 + i as u16;
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
            let row = input_row + 1 + drawn;
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

        // Park cursor on input line at end of text.
        let col = prompt_width
            + line_buf
                .chars()
                .take(line_cursor)
                .map(unicode_width)
                .sum::<usize>();
        let _ = crossterm::execute!(stdout, cursor::MoveTo(col as u16, input_row));
        let _ = stdout.flush();

        let Ok(event::Event::Key(key)) = event::read() else {
            continue;
        };

        match key.code {
            event::KeyCode::Up => {
                selected = selected.saturating_sub(1);
            }
            event::KeyCode::Down => {
                if !filtered.is_empty() {
                    selected = (selected + 1).min(filtered.len() - 1);
                }
            }
            event::KeyCode::Enter | event::KeyCode::Tab => {
                let result = filtered.get(selected).map(|s| s.to_string());
                erase_below(stdout, max_drawn, input_row);
                let col = prompt_width
                    + line_buf
                        .chars()
                        .take(line_cursor)
                        .map(unicode_width)
                        .sum::<usize>();
                let _ = crossterm::execute!(stdout, cursor::MoveTo(col as u16, input_row));
                let _ = stdout.flush();
                return result;
            }
            event::KeyCode::Esc => {
                erase_below(stdout, max_drawn, input_row);
                let col = prompt_width
                    + line_buf
                        .chars()
                        .take(line_cursor)
                        .map(unicode_width)
                        .sum::<usize>();
                let _ = crossterm::execute!(stdout, cursor::MoveTo(col as u16, input_row));
                let _ = stdout.flush();
                return None;
            }
            event::KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                erase_below(stdout, max_drawn, input_row);
                let col = prompt_width
                    + line_buf
                        .chars()
                        .take(line_cursor)
                        .map(unicode_width)
                        .sum::<usize>();
                let _ = crossterm::execute!(stdout, cursor::MoveTo(col as u16, input_row));
                let _ = stdout.flush();
                return None;
            }
            event::KeyCode::Backspace => {
                if filter.pop().is_some() {
                    // Also update the line buffer.
                    if line_cursor > 1 {
                        tui::handle_text_input(
                            event::KeyCode::Backspace,
                            &mut line_buf,
                            &mut line_cursor,
                        );
                    }
                    filtered = candidates
                        .iter()
                        .filter(|c| c.contains(filter.as_str()))
                        .map(|s| s.as_str())
                        .collect();
                    selected = 0;
                    scroll = 0;
                }
            }
            event::KeyCode::Char(' ') => {
                // Space exits the dropdown, keeping current input.
                // The caller adds a trailing space via `format!("{completed} ")`.
                erase_below(stdout, max_drawn, input_row);
                let col = prompt_width
                    + line_buf
                        .chars()
                        .take(line_cursor)
                        .map(unicode_width)
                        .sum::<usize>();
                let _ = crossterm::execute!(stdout, cursor::MoveTo(col as u16, input_row));
                let _ = stdout.flush();
                return Some(line_buf);
            }
            event::KeyCode::Char(ch) => {
                filter.push(ch);
                // Also append to the line buffer so the user sees what they type.
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

/// Clear all dropdown lines below the input row.
fn erase_below(stdout: &mut std::io::Stdout, max_drawn: u16, input_row: u16) {
    for i in 0..max_drawn {
        let _ = crossterm::execute!(
            stdout,
            cursor::MoveTo(0, input_row + 1 + i),
            Clear(ClearType::CurrentLine),
        );
    }
}

/// Approximate display width of a character.
fn unicode_width(c: char) -> usize {
    if c.is_ascii() { 1 } else { 2 }
}
