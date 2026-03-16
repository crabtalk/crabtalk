//! Task tab key handling and rendering.

use crate::cmd::console::{ConsoleState, Focus};
use crate::tui::{self, border_focused, format_duration, handle_text_input};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

pub(super) async fn handle_approve_input(
    key: crossterm::event::KeyEvent,
    state: &mut ConsoleState,
) {
    match key.code {
        crossterm::event::KeyCode::Esc => {
            state.focus = Focus::List;
        }
        crossterm::event::KeyCode::Enter => {
            if let Some(t) = state.tasks.get(state.selected) {
                let id = t.id;
                let response = state.edit_buf.clone();
                match state.runner.approve_task(id, response).await {
                    Ok(true) => state.status = format!("Task {id} approved"),
                    Ok(false) => state.status = format!("Task {id} not blocked"),
                    Err(e) => state.status = format!("Error: {e}"),
                }
            }
            state.focus = Focus::List;
            state.refresh().await;
        }
        _ => handle_text_input(key.code, &mut state.edit_buf, &mut state.cursor),
    }
}

pub(super) fn render_tasks(frame: &mut Frame, state: &ConsoleState, area: Rect) {
    let horiz = if state.focus == Focus::Approve {
        Layout::vertical([Constraint::Min(4), Constraint::Length(3)]).split(area)
    } else {
        Layout::vertical([Constraint::Min(4)]).split(area)
    };

    let block = Block::default()
        .title(" Tasks ")
        .borders(Borders::ALL)
        .border_style(border_focused());

    if state.tasks.is_empty() {
        frame.render_widget(Paragraph::new("  No active tasks.").block(block), horiz[0]);
    } else {
        let mut lines = vec![Line::from(vec![Span::styled(
            format!(
                "  {:<6} {:<16} {:<12} {:<10} {:<10}",
                "ID", "AGENT", "STATUS", "ALIVE", "TOKENS"
            ),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )])];

        for (i, t) in state.tasks.iter().enumerate() {
            let is_selected = i == state.selected;
            let marker = if is_selected { "> " } else { "  " };
            let alive = format_duration(t.alive_secs);
            let tokens = t.prompt_tokens + t.completion_tokens;
            let text = format!(
                "{marker}{:<6} {:<16} {:<12} {:<10} {:<10}",
                t.id, t.agent, t.status, alive, tokens
            );
            let style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(Span::styled(text, style)));

            if let Some(q) = &t.blocked_on {
                let blocked_style = Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::ITALIC);
                lines.push(Line::from(Span::styled(
                    format!("         blocked: {q}"),
                    blocked_style,
                )));
            }
        }

        frame.render_widget(Paragraph::new(lines).block(block), horiz[0]);
    }

    // Approve input area.
    if state.focus == Focus::Approve && horiz.len() > 1 {
        let block = Block::default()
            .title(" Approve Response ")
            .borders(Borders::ALL)
            .border_style(border_focused());
        let inner = block.inner(horiz[1]);
        frame.render_widget(block, horiz[1]);

        let byte_pos = tui::char_to_byte(&state.edit_buf, state.cursor);
        let mut s = state.edit_buf.clone();
        s.insert(byte_pos, '|');
        frame.render_widget(
            Paragraph::new(Span::styled(s, Style::default().fg(Color::Green))),
            inner,
        );
    }
}
