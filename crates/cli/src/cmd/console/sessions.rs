//! Session browser — identity list and conversation drill-down.

use crate::tui::border_focused;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use std::{collections::BTreeMap, fs, path::Path};

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
    #[allow(dead_code)]
    pub seq: u32,
    pub title: String,
    /// File path for this conversation (used for resume).
    #[allow(dead_code)]
    pub file_path: std::path::PathBuf,
    /// Message count (from disk line count or daemon).
    pub message_count: Option<u64>,
    /// Uptime in seconds (from disk meta or daemon).
    pub alive_secs: Option<u64>,
    /// Daemon session ID (for correlating with live events).
    pub session_id: Option<u64>,
}

impl SessionView {
    /// Refresh identity list from disk, merged with daemon live data.
    pub fn refresh_identities(
        &mut self,
        daemon_sessions: &[wcore::protocol::message::SessionInfo],
    ) {
        let mut entries = scan_identities(&wcore::paths::SESSIONS_DIR);

        // Merge daemon live data into identity entries.
        for entry in &mut entries {
            for ds in daemon_sessions {
                if ds.agent == entry.agent && ds.created_by == entry.sender {
                    entry.message_count += ds.message_count;
                    entry.alive_secs = entry.alive_secs.max(ds.alive_secs);
                    // Session is loaded in memory — it's active today.
                    entry.last_active = "Today".to_string();
                }
            }
        }

        let selected = match self {
            Self::Identities { selected, .. } => (*selected).min(entries.len().saturating_sub(1)),
            _ => 0,
        };
        *self = Self::Identities { entries, selected };
    }

    /// Enter the selected identity to show its conversations.
    /// `daemon_sessions` provides live session info from the daemon.
    pub fn enter(&mut self, daemon_sessions: &[wcore::protocol::message::SessionInfo]) {
        if let Self::Identities { entries, selected } = self
            && let Some(entry) = entries.get(*selected)
        {
            let mut conversations =
                scan_conversations(&wcore::paths::SESSIONS_DIR, &entry.agent, &entry.sender);

            // Merge live stats from daemon sessions that match this identity.
            for ds in daemon_sessions {
                if ds.agent == entry.agent && ds.created_by == entry.sender {
                    // Match by title: the daemon session's title slug should match
                    // one of the conversation entries' title.
                    let title_slug = wcore::sender_slug(&ds.title);
                    if let Some(conv) = conversations.iter_mut().find(|c| {
                        if ds.title.is_empty() && c.title.is_empty() {
                            true // both untitled — match the latest
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
                entries: conversations,
                selected: 0,
            };
        }
    }

    /// Update live stats from daemon data without resetting selection.
    pub fn merge_daemon_data(&mut self, daemon_sessions: &[wcore::protocol::message::SessionInfo]) {
        match self {
            Self::Identities { entries, .. } => {
                // Reset live stats, then re-merge.
                for e in entries.iter_mut() {
                    e.message_count = 0;
                    e.alive_secs = 0;
                }
                for e in entries.iter_mut() {
                    for ds in daemon_sessions {
                        if ds.agent == e.agent && ds.created_by == e.sender {
                            e.message_count += ds.message_count;
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
                // Reset live stats, then re-merge for this identity.
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
            entries.get(*selected).map(|e| e.file_path.clone())
        } else {
            None
        }
    }

    /// Go back to identity list.
    pub fn back(&mut self, daemon_sessions: &[wcore::protocol::message::SessionInfo]) {
        if matches!(self, Self::Conversations { .. }) {
            self.refresh_identities(daemon_sessions);
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

/// Convert a file mtime to a display label relative to today.
fn mtime_to_label(mtime: std::time::SystemTime, today: chrono::NaiveDate) -> String {
    let date = chrono::DateTime::<chrono::Local>::from(mtime).date_naive();
    if date == today {
        "Today".to_string()
    } else if date == today - chrono::Duration::days(1) {
        "Yesterday".to_string()
    } else {
        date.format("%Y-%m-%d").to_string()
    }
}

// ── Filesystem scanning ─────────────────────────────────────────────

/// Scan flat sessions directory and return unique identities with stats.
fn scan_identities(sessions_dir: &Path) -> Vec<IdentityEntry> {
    // Track: (agent, sender) → (count, latest_mtime, total_uptime, total_msgs)
    let mut data: BTreeMap<(String, String), (usize, std::time::SystemTime, u64, u64)> =
        BTreeMap::new();

    let Ok(entries) = fs::read_dir(sessions_dir) else {
        return Vec::new();
    };

    for file in entries.flatten() {
        let path = file.path();
        if path.is_dir() {
            continue;
        }
        let name = file.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.ends_with(".jsonl") {
            continue;
        }
        if let Some((agent, sender)) = parse_identity_from_filename(name) {
            let mtime = file
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let (uptime, msgs) = read_file_stats(&path);
            let entry =
                data.entry((agent, sender))
                    .or_insert((0, std::time::SystemTime::UNIX_EPOCH, 0, 0));
            entry.0 += 1;
            if mtime > entry.1 {
                entry.1 = mtime;
            }
            entry.2 += uptime;
            entry.3 += msgs;
        }
    }

    let today = chrono::Local::now().date_naive();
    let mut entries: Vec<_> = data
        .into_iter()
        .map(|((agent, sender), (count, mtime, uptime, msgs))| {
            let last_active = mtime_to_label(mtime, today);
            (
                mtime,
                IdentityEntry {
                    agent,
                    sender,
                    count,
                    message_count: msgs,
                    last_active,
                    alive_secs: uptime,
                },
            )
        })
        .collect();
    // Sort by mtime descending (most recently active first).
    entries.sort_by(|a, b| b.0.cmp(&a.0));
    entries.into_iter().map(|(_, e)| e).collect()
}

/// Scan conversations for a specific identity, sorted by mtime (newest first).
fn scan_conversations(sessions_dir: &Path, agent: &str, sender: &str) -> Vec<ConversationEntry> {
    let slug = wcore::sender_slug(sender);
    let prefix = format!("{agent}_{slug}_");
    let today = chrono::Local::now().date_naive();

    let Ok(files) = fs::read_dir(sessions_dir) else {
        return Vec::new();
    };

    let mut raw: Vec<(
        std::time::SystemTime,
        u32,
        String,
        std::path::PathBuf,
        u64,
        u64,
    )> = Vec::new();
    for file in files.flatten() {
        let path = file.path();
        if path.is_dir() {
            continue;
        }
        let name = file.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(&prefix) || !name.ends_with(".jsonl") {
            continue;
        }
        let after_prefix = &name[prefix.len()..name.len() - 6]; // strip .jsonl
        let (seq, title) = if let Some(underscore) = after_prefix.find('_') {
            let seq: u32 = after_prefix[..underscore].parse().unwrap_or(0);
            let title = after_prefix[underscore + 1..].to_string();
            (seq, title)
        } else {
            let seq: u32 = after_prefix.parse().unwrap_or(0);
            (seq, String::new())
        };
        let mtime = file
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

        // Read stats from the file: meta line for uptime, line count for messages.
        let (uptime, msg_count) = read_file_stats(&path);
        raw.push((mtime, seq, title, path, uptime, msg_count));
    }

    // Sort by mtime descending (newest first).
    raw.sort_by(|a, b| b.0.cmp(&a.0));

    raw.into_iter()
        .map(
            |(mtime, seq, title, file_path, uptime, msg_count)| ConversationEntry {
                date: mtime_to_label(mtime, today),
                seq,
                title,
                file_path,
                message_count: Some(msg_count),
                alive_secs: Some(uptime),
                session_id: None,
            },
        )
        .collect()
}

/// Read uptime_secs from meta line and count message lines from a session file.
/// Returns (uptime_secs, message_count).
fn read_file_stats(path: &Path) -> (u64, u64) {
    use std::io::{BufRead, BufReader};

    let Ok(file) = fs::File::open(path) else {
        return (0, 0);
    };
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    // First line is meta — extract uptime_secs.
    let uptime = lines
        .next()
        .and_then(|l| l.ok())
        .and_then(|l| {
            let v: serde_json::Value = serde_json::from_str(&l).ok()?;
            v.get("uptime_secs")?.as_u64()
        })
        .unwrap_or(0);

    // Count remaining non-empty, non-compact lines as messages.
    let msg_count = lines
        .map_while(|l| l.ok())
        .filter(|l| !l.trim().is_empty() && !l.contains("\"compact\""))
        .count() as u64;

    (uptime, msg_count)
}

/// Parse agent and sender from a session filename.
/// Format: `{agent}_{sender}_{seq}[_{title}].jsonl`
fn parse_identity_from_filename(name: &str) -> Option<(String, String)> {
    let stem = name.strip_suffix(".jsonl")?;
    // Split by '_' and find the first part that looks like a seq number.
    // Everything before the seq is agent_sender.
    let parts: Vec<&str> = stem.split('_').collect();
    if parts.len() < 3 {
        return None;
    }
    // Find the first numeric part (that's the seq).
    for i in 2..parts.len() {
        if parts[i].chars().all(|c| c.is_ascii_digit()) && !parts[i].is_empty() {
            let agent = parts[0].to_string();
            let sender = parts[1..i].join("_");
            return Some((agent, sender));
        }
    }
    None
}
