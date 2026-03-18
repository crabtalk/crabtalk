//! Task tab rendering.

use crate::cmd::console::ConsoleState;
use crate::tui::{border_focused, format_duration};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

pub(super) fn render_tasks(frame: &mut Frame, state: &ConsoleState, area: Rect) {
    let block = Block::default()
        .title(" Tasks ")
        .borders(Borders::ALL)
        .border_style(border_focused());

    if state.tasks.is_empty() {
        frame.render_widget(Paragraph::new("  No active tasks.").block(block), area);
    } else {
        let mut lines = vec![Line::from(vec![Span::styled(
            format!(
                "  {:<6} {:<16} {:<12} {:<10}",
                "ID", "AGENT", "STATUS", "ALIVE"
            ),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )])];

        for (i, t) in state.tasks.iter().enumerate() {
            let is_selected = i == state.selected;
            let marker = if is_selected { "> " } else { "  " };
            let alive = format_duration(t.alive_secs);
            let text = format!(
                "{marker}{:<6} {:<16} {:<12} {:<10}",
                t.id, t.agent, t.status, alive
            );
            let style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(Span::styled(text, style)));
        }

        frame.render_widget(Paragraph::new(lines).block(block), area);
    }
}
