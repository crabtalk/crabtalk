//! Session tab rendering.

use crate::cmd::console::ConsoleState;
use crate::tui::{border_focused, format_duration};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

pub(super) fn render_sessions(frame: &mut Frame, state: &ConsoleState, area: Rect) {
    let block = Block::default()
        .title(" Sessions ")
        .borders(Borders::ALL)
        .border_style(border_focused());

    if state.sessions.is_empty() {
        frame.render_widget(Paragraph::new("  No active sessions.").block(block), area);
        return;
    }

    let mut lines = vec![Line::from(vec![Span::styled(
        format!(
            "  {:<6} {:<16} {:<16} {:<8} {:<8} {:<10}",
            "ID", "AGENT", "CREATED BY", "MSGS", "STATUS", "UPTIME"
        ),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )])];

    for (i, s) in state.sessions.iter().enumerate() {
        let is_selected = i == state.selected;
        let marker = if is_selected { "> " } else { "  " };
        let alive = format_duration(s.alive_secs);
        let status = if s.active { "active" } else { "idle" };
        let text = format!(
            "{marker}{:<6} {:<16} {:<16} {:<8} {:<8} {:<10}",
            s.id, s.agent, s.created_by, s.message_count, status, alive
        );
        let style = if is_selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else if s.active {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        lines.push(Line::from(Span::styled(text, style)));
    }

    frame.render_widget(Paragraph::new(lines).block(block), area);
}
