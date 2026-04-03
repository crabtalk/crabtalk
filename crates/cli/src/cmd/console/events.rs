//! Events tab rendering.

use crate::tui::border_focused;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use wcore::protocol::message::{AgentEventKind, AgentEventMsg};

const CONVERSATION_COLORS: &[Color] = &[
    Color::LightMagenta,
    Color::LightCyan,
    Color::LightGreen,
    Color::LightYellow,
    Color::LightBlue,
    Color::LightRed,
];

fn conversation_color(conversation_id: u64) -> Color {
    CONVERSATION_COLORS[conversation_id as usize % CONVERSATION_COLORS.len()]
}

pub(super) struct EventEntry {
    pub(super) timestamp: String,
    pub(super) msg: AgentEventMsg,
}

pub(super) fn render_events(
    frame: &mut Frame,
    events: &[&EventEntry],
    scroll_offset: usize,
    area: Rect,
) {
    let block = Block::default()
        .title(" Events ")
        .borders(Borders::ALL)
        .border_style(border_focused());

    let filtered: Vec<&&EventEntry> = events
        .iter()
        .filter(|e| {
            matches!(
                AgentEventKind::try_from(e.msg.kind),
                Ok(AgentEventKind::ToolStart | AgentEventKind::ToolResult | AgentEventKind::Done)
            )
        })
        .collect();

    if filtered.is_empty() {
        frame.render_widget(
            Paragraph::new("  No events yet. Waiting for agent activity...").block(block),
            area,
        );
        return;
    }

    let inner_height = area.height.saturating_sub(2) as usize;
    // Newest first: reverse, then skip/take for scrolling.
    let lines: Vec<Line> = filtered
        .iter()
        .rev()
        .skip(scroll_offset)
        .take(inner_height)
        .map(|entry| {
            let (kind_str, kind_color) = match AgentEventKind::try_from(entry.msg.kind) {
                Ok(AgentEventKind::ToolStart) => ("TOOL_CALL", Color::Rgb(215, 119, 87)),
                Ok(AgentEventKind::ToolResult) => ("TOOL_DONE", Color::Rgb(87, 187, 138)),
                Ok(AgentEventKind::Done) => ("DONE", Color::Rgb(87, 187, 138)),
                _ => ("EVENT", Color::DarkGray),
            };
            let content_part = if entry.msg.content.is_empty() {
                String::new()
            } else {
                let c = &entry.msg.content;
                let display = if c.len() > 60 {
                    format!("{}...", &c[..57])
                } else {
                    c.clone()
                };
                format!(": {display}")
            };
            let conversation_color = conversation_color(entry.msg.session);
            Line::from(vec![
                Span::styled(
                    format!("  [{}] ", entry.timestamp),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("{}({}) ", entry.msg.agent, entry.msg.session),
                    Style::default()
                        .fg(conversation_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{kind_str}{content_part}"),
                    Style::default().fg(kind_color),
                ),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines).block(block), area);
}
