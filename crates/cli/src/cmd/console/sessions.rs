//! Session browser — identity list and conversation drill-down.

use crate::tui::border_focused;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use std::collections::BTreeMap;
use wcore::protocol::message::{ConversationInfo, SessionInfo};

/// Which view the Sessions tab is showing.
#[derive(Clone)]
pub(super) enum SessionView {
    /// Top-level identity list.
    Identities {
        entries: Vec<IdentityEntry>,
        selected: usize,
    },
    /// Drill-down: conversations for one identity.
    Conversations {
        agent: String,
        sender: String,
        entries: Vec<ConversationEntry>,
        selected: usize,
    },
}

impl Default for SessionView {
    fn default() -> Self {
        Self::Identities {
            entries: Vec::new(),
            selected: 0,
        }
    }
}

#[derive(Clone)]
pub(super) struct IdentityEntry {
    pub agent: String,
    pub sender: String,
    pub count: usize,
    pub message_count: u64,
    pub last_active: String,
    pub alive_secs: u64,
}

#[derive(Clone)]
pub(super) struct ConversationEntry {
    pub date: String,
    pub title: String,
    /// File path for this conversation (used for resume).
    pub file_path: String,
    /// Message count (from daemon).
    pub message_count: Option<u64>,
    /// Uptime in seconds (from daemon).
    pub alive_secs: Option<u64>,
    /// Daemon session ID (for correlating with live events).
    pub session_id: Option<u64>,
}

impl SessionView {
    /// Refresh identity list from daemon data.
    pub fn refresh_identities(
        &mut self,
        conversations: &[ConversationInfo],
        daemon_sessions: &[SessionInfo],
    ) {
        let mut data: BTreeMap<(String, String), (usize, String, u64, u64)> = BTreeMap::new();

        for c in conversations {
            let key = (c.agent.clone(), c.sender.clone());
            let entry = data.entry(key).or_insert((0, String::new(), 0, 0));
            entry.0 += 1;
            // Keep the most recent date label.
            if entry.1.is_empty()
                || c.date == "Today"
                || (c.date == "Yesterday" && entry.1 != "Today")
            {
                entry.1.clone_from(&c.date);
            }
            entry.2 += c.alive_secs;
            entry.3 += c.message_count;
        }

        // Merge live daemon session data.
        for ds in daemon_sessions {
            let key = (ds.agent.clone(), ds.created_by.clone());
            let entry = data.entry(key).or_insert((0, String::new(), 0, 0));
            entry.1 = "Today".to_string();
            entry.2 = entry.2.max(ds.alive_secs);
            entry.3 = entry.3.max(ds.message_count);
        }

        let mut entries: Vec<_> = data
            .into_iter()
            .map(
                |((agent, sender), (count, last_active, alive_secs, message_count))| {
                    IdentityEntry {
                        agent,
                        sender,
                        count,
                        message_count,
                        last_active,
                        alive_secs,
                    }
                },
            )
            .collect();
        // Sort: active today first, then by name.
        entries.sort_by(|a, b| {
            let a_today = a.last_active == "Today";
            let b_today = b.last_active == "Today";
            b_today.cmp(&a_today).then(a.agent.cmp(&b.agent))
        });

        let selected = match self {
            Self::Identities { selected, .. } => (*selected).min(entries.len().saturating_sub(1)),
            _ => 0,
        };
        *self = Self::Identities { entries, selected };
    }

    /// Enter the selected identity to show its conversations.
    pub fn enter(&mut self, conversations: &[ConversationInfo], daemon_sessions: &[SessionInfo]) {
        if let Self::Identities { entries, selected } = self
            && let Some(entry) = entries.get(*selected)
        {
            let mut conv_entries: Vec<ConversationEntry> = conversations
                .iter()
                .map(|c| ConversationEntry {
                    date: c.date.clone(),
                    title: c.title.clone(),
                    file_path: c.file_path.clone(),
                    message_count: Some(c.message_count),
                    alive_secs: Some(c.alive_secs),
                    session_id: None,
                })
                .collect();

            // Merge live stats from daemon sessions.
            for ds in daemon_sessions {
                if ds.agent == entry.agent && ds.created_by == entry.sender {
                    let title_slug = wcore::sender_slug(&ds.title);
                    if let Some(conv) = conv_entries.iter_mut().find(|c| {
                        if ds.title.is_empty() && c.title.is_empty() {
                            true
                        } else {
                            c.title == title_slug
                        }
                    }) {
                        conv.message_count = Some(ds.message_count);
                        conv.alive_secs = Some(ds.alive_secs);
                        conv.session_id = Some(ds.id);
                    }
                }
            }

            *self = Self::Conversations {
                agent: entry.agent.clone(),
                sender: entry.sender.clone(),
                entries: conv_entries,
                selected: 0,
            };
        }
    }

    /// Update live stats from daemon data without resetting selection.
    /// Only overlays live session info — does not touch base counts from
    /// the last `refresh_identities` call.
    pub fn merge_daemon_data(&mut self, daemon_sessions: &[SessionInfo]) {
        match self {
            Self::Identities { entries, .. } => {
                for e in entries.iter_mut() {
                    for ds in daemon_sessions {
                        if ds.agent == e.agent && ds.created_by == e.sender {
                            e.message_count = e.message_count.max(ds.message_count);
                            e.alive_secs = e.alive_secs.max(ds.alive_secs);
                            e.last_active = "Today".to_string();
                        }
                    }
                }
            }
            Self::Conversations {
                agent,
                sender,
                entries,
                ..
            } => {
                for c in entries.iter_mut() {
                    c.message_count = None;
                    c.alive_secs = None;
                    c.session_id = None;
                }
                for ds in daemon_sessions {
                    if ds.agent.as_str() == agent.as_str()
                        && ds.created_by.as_str() == sender.as_str()
                    {
                        let title_slug = wcore::sender_slug(&ds.title);
                        if let Some(conv) = entries.iter_mut().find(|c| {
                            if ds.title.is_empty() && c.title.is_empty() {
                                true
                            } else {
                                c.title == title_slug
                            }
                        }) {
                            conv.message_count = Some(ds.message_count);
                            conv.alive_secs = Some(ds.alive_secs);
                            conv.session_id = Some(ds.id);
                        }
                    }
                }
            }
        }
    }

    /// Get the file path of the currently selected conversation (if in conversation view).
    pub fn selected_file(&self) -> Option<std::path::PathBuf> {
        if let Self::Conversations {
            entries, selected, ..
        } = self
        {
            entries
                .get(*selected)
                .map(|e| std::path::PathBuf::from(&e.file_path))
        } else {
            None
        }
    }

    /// Get the (agent, sender) of the currently selected identity.
    pub fn selected_identity(&self) -> Option<(&str, &str)> {
        if let Self::Identities { entries, selected } = self {
            entries
                .get(*selected)
                .map(|e| (e.agent.as_str(), e.sender.as_str()))
        } else {
            None
        }
    }

    /// Go back to identity list.
    pub fn back(&mut self, conversations: &[ConversationInfo], daemon_sessions: &[SessionInfo]) {
        if matches!(self, Self::Conversations { .. }) {
            self.refresh_identities(conversations, daemon_sessions);
        }
    }

    pub fn move_up(&mut self) {
        match self {
            Self::Identities { selected, .. } | Self::Conversations { selected, .. } => {
                *selected = selected.saturating_sub(1);
            }
        }
    }

    pub fn move_down(&mut self) {
        match self {
            Self::Identities { entries, selected } => {
                if !entries.is_empty() {
                    *selected = (*selected + 1).min(entries.len() - 1);
                }
            }
            Self::Conversations {
                entries, selected, ..
            } => {
                if !entries.is_empty() {
                    *selected = (*selected + 1).min(entries.len() - 1);
                }
            }
        }
    }
}

// ── Rendering ────────────────────────────────────────────────────────

pub(super) fn render_session_view(frame: &mut Frame, view: &SessionView, area: Rect) {
    match view {
        SessionView::Identities { entries, selected } => {
            render_identities(frame, entries, *selected, area);
        }
        SessionView::Conversations {
            agent,
            sender,
            entries,
            selected,
        } => {
            render_conversations(frame, agent, sender, entries, *selected, area);
        }
    }
}

fn render_identities(frame: &mut Frame, entries: &[IdentityEntry], selected: usize, area: Rect) {
    let block = Block::default()
        .title(" Sessions ")
        .borders(Borders::ALL)
        .border_style(border_focused());

    if entries.is_empty() {
        frame.render_widget(
            Paragraph::new("  No sessions found. Start a conversation first.").block(block),
            area,
        );
        return;
    }

    let mut lines = vec![Line::from(vec![Span::styled(
        format!(
            "  {:<24} {:<8} {:<8} {:<14} {:<10}",
            "IDENTITY", "CHATS", "MSGS", "LAST ACTIVE", "UPTIME"
        ),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )])];

    for (i, e) in entries.iter().enumerate() {
        let is_selected = i == selected;
        let marker = if is_selected { "> " } else { "  " };
        let identity = format!("{}({})", e.sender, e.agent);
        let msgs = if e.message_count > 0 {
            e.message_count.to_string()
        } else {
            "—".to_string()
        };
        let uptime = if e.alive_secs > 0 {
            crate::tui::format_duration(e.alive_secs)
        } else {
            "—".to_string()
        };
        let text = format!(
            "{marker}{:<24} {:<8} {:<8} {:<14} {:<10}",
            identity, e.count, msgs, e.last_active, uptime
        );
        let style = if is_selected {
            Style::default()
                .fg(Color::Rgb(215, 119, 87))
                .add_modifier(Modifier::BOLD)
        } else if e.alive_secs > 0 {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        lines.push(Line::from(Span::styled(text, style)));
    }

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_conversations(
    frame: &mut Frame,
    agent: &str,
    sender: &str,
    entries: &[ConversationEntry],
    selected: usize,
    area: Rect,
) {
    let title = format!(" {sender}({agent}) — {} conversations ", entries.len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_focused());

    if entries.is_empty() {
        frame.render_widget(
            Paragraph::new("  No conversations found.").block(block),
            area,
        );
        return;
    }

    // Table header.
    let mut lines = vec![Line::from(vec![Span::styled(
        format!(
            "  {:<14} {:<26} {:<8} {:<8} {:<10}",
            "LAST ACTIVE", "TITLE", "MSGS", "SID", "UPTIME"
        ),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )])];

    for (i, e) in entries.iter().enumerate() {
        let is_selected = i == selected;
        let marker = if is_selected { "> " } else { "  " };
        let title_display: String = if e.title.is_empty() {
            "(untitled)".to_string()
        } else {
            e.title.chars().take(24).collect()
        };
        let msgs = e
            .message_count
            .map(|n| n.to_string())
            .unwrap_or_else(|| "—".to_string());
        let sid = e
            .session_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "—".to_string());
        let uptime = e
            .alive_secs
            .map(crate::tui::format_duration)
            .unwrap_or_else(|| "—".to_string());
        let text = format!(
            "{marker}{:<14} {:<26} {:<8} {:<8} {:<10}",
            e.date, title_display, msgs, sid, uptime
        );
        let style = if is_selected {
            Style::default()
                .fg(Color::Rgb(215, 119, 87))
                .add_modifier(Modifier::BOLD)
        } else if e.alive_secs.is_some() {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        lines.push(Line::from(Span::styled(text, style)));
    }

    frame.render_widget(Paragraph::new(lines).block(block), area);
}
