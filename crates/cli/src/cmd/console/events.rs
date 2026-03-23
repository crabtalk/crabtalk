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

    if events.is_empty() {
        frame.render_widget(
            Paragraph::new("  No events yet. Waiting for agent activity...").block(block),
            area,
        );
        return;
    }

    let inner_height = area.height.saturating_sub(2) as usize;
    let lines: Vec<Line> = events
        .iter()
        .skip(scroll_offset)
        .take(inner_height)
        .map(|entry| {
            let kind_str = match AgentEventKind::try_from(entry.msg.kind) {
                Ok(AgentEventKind::TextDelta) => "TEXT",
                Ok(AgentEventKind::ThinkingDelta) => "THINK",
                Ok(AgentEventKind::ToolStart) => "TOOL_START",
                Ok(AgentEventKind::ToolResult) => "TOOL_RESULT",
                Ok(AgentEventKind::ToolsComplete) => "TOOLS_DONE",
                Ok(AgentEventKind::Done) => "DONE",
                Err(_) => "UNKNOWN",
            };
            let content_part = if entry.msg.content.is_empty() {
                String::new()
            } else {
                format!(": {}", entry.msg.content)
            };
            let kind_color = match AgentEventKind::try_from(entry.msg.kind) {
                Ok(AgentEventKind::ToolStart) => Color::Yellow,
                Ok(AgentEventKind::Done) => Color::Green,
                Ok(AgentEventKind::ToolResult) => Color::Cyan,
                _ => Color::White,
            };
            Line::from(vec![
                Span::styled(
                    format!("  [{}] ", entry.timestamp),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("{}#{} ", entry.msg.agent, entry.msg.session),
                    Style::default()
                        .fg(Color::LightMagenta)
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
