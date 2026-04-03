//! Reusable TUI components for crabtalk CLI screens.

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    style::{Color, Style},
};
use std::io::Stdout;

/// Run a full-screen TUI application.
///
/// Handles terminal setup, the event loop, and teardown (including panic
/// recovery). `init` creates the initial state, and each iteration calls
/// `draw` then `handle_key`. The loop exits when `handle_key` returns
/// `Some(result)`.
pub fn run_app<S>(
    init: impl FnOnce() -> Result<S>,
    draw: impl Fn(&mut ratatui::Frame, &S),
    handle_key: impl Fn(event::KeyEvent, &mut S) -> Result<Option<Result<()>>>,
) -> Result<()> {
    let mut terminal = setup()?;
    let mut state = init()?;
    let result = event_loop(&mut terminal, &mut state, &draw, &handle_key);
    teardown(&mut terminal)?;
    result
}

/// Run a TUI application, returning the final state on exit.
///
/// Same as [`run_app`] but hands the state back to the caller so it can
/// inspect flags set during the run (e.g. `pending_login`).
pub fn run_app_with_state<S>(
    init: impl FnOnce() -> Result<S>,
    draw: impl Fn(&mut ratatui::Frame, &S),
    handle_key: impl Fn(event::KeyEvent, &mut S) -> Result<Option<Result<()>>>,
) -> Result<S> {
    let mut terminal = setup()?;
    let mut state = init()?;
    let result = event_loop(&mut terminal, &mut state, &draw, &handle_key);
    teardown(&mut terminal)?;
    result?;
    Ok(state)
}

/// Prepare terminal for TUI. Returns the terminal handle.
/// Call `teardown` when done.
pub fn setup() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        original_hook(info);
    }));

    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

/// Restore terminal after TUI.
pub fn teardown(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Poll for a key event without blocking (250ms timeout).
/// Returns `Some(key)` if a key was pressed, `None` on timeout.
pub fn poll_key() -> Result<Option<event::KeyEvent>> {
    if event::poll(std::time::Duration::from_millis(250))?
        && let Event::Key(key) = event::read()?
    {
        Ok(Some(key))
    } else {
        Ok(None)
    }
}

fn event_loop<S>(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    state: &mut S,
    draw: &impl Fn(&mut ratatui::Frame, &S),
    handle_key: &impl Fn(event::KeyEvent, &mut S) -> Result<Option<Result<()>>>,
) -> Result<()> {
    loop {
        terminal.draw(|frame| draw(frame, state))?;
        if event::poll(std::time::Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
            && let Some(result) = handle_key(key, state)?
        {
            return result;
        }
    }
}

// ── Text editing helpers ────────────────────────────────────────────

/// Convert a char index to a byte offset within a string.
pub fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

/// Handle standard text-input key events on a buffer + cursor.
pub fn handle_text_input(code: KeyCode, buf: &mut String, cursor: &mut usize) {
    match code {
        KeyCode::Backspace => {
            if *cursor > 0 {
                let start = char_to_byte(buf, *cursor - 1);
                let end = char_to_byte(buf, *cursor);
                buf.drain(start..end);
                *cursor -= 1;
            }
        }
        KeyCode::Delete => {
            let char_count = buf.chars().count();
            if *cursor < char_count {
                let start = char_to_byte(buf, *cursor);
                let end = char_to_byte(buf, *cursor + 1);
                buf.drain(start..end);
            }
        }
        KeyCode::Left => {
            *cursor = cursor.saturating_sub(1);
        }
        KeyCode::Right => {
            let char_count = buf.chars().count();
            if *cursor < char_count {
                *cursor += 1;
            }
        }
        KeyCode::Home => {
            *cursor = 0;
        }
        KeyCode::End => {
            *cursor = buf.chars().count();
        }
        KeyCode::Char(c) => {
            let byte_pos = char_to_byte(buf, *cursor);
            buf.insert(byte_pos, c);
            *cursor += 1;
        }
        _ => {}
    }
}

/// Mask a token for display — show first 4 and last 4 chars.
pub fn mask_token(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() <= 8 {
        "*".repeat(chars.len())
    } else {
        let head: String = chars[..4].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}{}{tail}", "*".repeat(chars.len() - 8))
    }
}

// ── Style helpers ───────────────────────────────────────────────────

/// Border style for a focused panel (brand orange).
pub fn border_focused() -> Style {
    Style::default().fg(Color::Rgb(215, 119, 87))
}

/// Border style for an unfocused panel.
pub fn border_dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// Format seconds into a human-readable duration string.
pub fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}
