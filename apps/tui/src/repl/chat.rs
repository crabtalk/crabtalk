//! Chat buffer — structured storage for streaming chat output.
//!
//! Entries are appended by the [`super::render::MarkdownRenderer`] and
//! flattened into `Vec<Line>` for display in the chat area widget.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use termimad::{CompositeKind, FmtLine, FmtText, MadSkin};

// ── Brand colours (same palette as the old renderer) ─────────────

pub const BRAND: Color = Color::Indexed(173);
pub const GREEN: Color = Color::Indexed(71);
pub const RED: Color = Color::Indexed(204);
pub const SUBTLE: Color = Color::Indexed(240);

pub const S_BRAND: Style = Style::new().fg(BRAND);
pub const S_DIM: Style = Style::new().add_modifier(Modifier::DIM);
pub const S_SUBTLE: Style = Style::new().fg(SUBTLE);

// ── Data model ───────────────────────────────────────────────────

/// Status of a tool invocation (drives the marker colour).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Success,
    Failure,
}

/// One delegated task's live state within a fan-out.
#[derive(Debug, Clone)]
pub struct TaskProgress {
    pub agent: String,
    pub status: ToolStatus,
    /// Trailing text: what the sub-agent is doing while it runs, then its
    /// result headline or error once it settles.
    pub detail: String,
}

/// One logical chunk in the chat output.
#[derive(Debug, Clone)]
pub enum ChatEntry {
    /// Rendered markdown text (one or more display lines).
    Text(Vec<Line<'static>>),
    /// Tool call marker (`⏺ ToolName`).
    ToolMarker {
        labels: Vec<String>,
        status: ToolStatus,
    },
    /// Tool result output (`⎿ ...`).
    ToolResult(Vec<Line<'static>>),
    /// Live per-task status for one `delegate` fan-out. Keyed by the
    /// forwarded call so two concurrent fan-outs update independently.
    DelegateTasks {
        call_id: String,
        tasks: Vec<TaskProgress>,
    },
    /// Thinking / reasoning text (dimmed, italic).
    Thinking(Vec<Line<'static>>),
    /// Blank separator line.
    Blank,
}

/// Append-only buffer of chat entries.
#[derive(Debug, Default)]
pub struct ChatBuffer {
    pub entries: Vec<ChatEntry>,
}

impl ChatBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, entry: ChatEntry) {
        self.entries.push(entry);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Find the last `ToolMarker` that is still `Running` and flip its status.
    pub fn finish_tool(&mut self, success: bool) {
        let status = if success {
            ToolStatus::Success
        } else {
            ToolStatus::Failure
        };
        for entry in self.entries.iter_mut().rev() {
            if let ChatEntry::ToolMarker { status: s, .. } = entry
                && *s == ToolStatus::Running
            {
                *s = status;
                return;
            }
        }
    }

    /// Settle one task of the fan-out identified by `call_id`.
    pub fn finish_delegate_task(&mut self, call_id: &str, index: usize, ok: bool, detail: String) {
        let status = if ok {
            ToolStatus::Success
        } else {
            ToolStatus::Failure
        };
        if let Some(task) = self.delegate_task_mut(call_id, index) {
            task.status = status;
            task.detail = detail;
        }
    }

    /// Replace a running task's trailing text with what it's doing now.
    pub fn set_delegate_task_detail(&mut self, call_id: &str, index: usize, detail: String) {
        if let Some(task) = self.delegate_task_mut(call_id, index)
            && task.status == ToolStatus::Running
        {
            task.detail = detail;
        }
    }

    fn delegate_task_mut(&mut self, call_id: &str, index: usize) -> Option<&mut TaskProgress> {
        for entry in self.entries.iter_mut().rev() {
            if let ChatEntry::DelegateTasks { call_id: id, tasks } = entry
                && id == call_id
            {
                return tasks.get_mut(index);
            }
        }
        None
    }

    /// Settle every still-running task row. Called when a turn is
    /// cancelled — the `Finished` updates that would have settled them are
    /// never coming, and a row left `Running` spins forever.
    pub fn cancel_running_tasks(&mut self) {
        for entry in self.entries.iter_mut() {
            if let ChatEntry::DelegateTasks { tasks, .. } = entry {
                for task in tasks.iter_mut().filter(|t| t.status == ToolStatus::Running) {
                    task.status = ToolStatus::Failure;
                    task.detail = "cancelled".to_owned();
                }
            }
        }
    }

    /// Flatten all entries into display lines for the chat widget.
    ///
    /// `frame` drives the animation for running tool markers (pass the
    /// current frame counter from the event loop).
    pub fn lines(&self, frame: u64) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        for entry in &self.entries {
            match entry {
                ChatEntry::Text(lines) => out.extend(lines.iter().cloned()),
                ChatEntry::ToolMarker { labels, status } => {
                    let (marker, label_style) = match status {
                        ToolStatus::Running => (
                            Span::styled(spinner(frame), Style::new().add_modifier(Modifier::DIM)),
                            Style::new().add_modifier(Modifier::BOLD | Modifier::DIM),
                        ),
                        ToolStatus::Success => (
                            Span::styled("⏺ ", Style::new().fg(GREEN)),
                            Style::new().add_modifier(Modifier::BOLD | Modifier::DIM),
                        ),
                        ToolStatus::Failure => (
                            Span::styled("⏺ ", Style::new().fg(RED)),
                            Style::new().add_modifier(Modifier::BOLD | Modifier::DIM),
                        ),
                    };
                    for label in labels {
                        out.push(Line::from(vec![
                            marker.clone(),
                            Span::styled(label.clone(), label_style),
                        ]));
                    }
                }
                ChatEntry::ToolResult(lines) => out.extend(lines.iter().cloned()),
                ChatEntry::DelegateTasks { tasks, .. } => {
                    for task in tasks {
                        let (marker, detail_style) = match task.status {
                            ToolStatus::Running => (
                                Span::styled(
                                    spinner(frame),
                                    Style::new().add_modifier(Modifier::DIM),
                                ),
                                S_DIM,
                            ),
                            ToolStatus::Success => {
                                (Span::styled("⏺ ", Style::new().fg(GREEN)), S_DIM)
                            }
                            ToolStatus::Failure => (
                                Span::styled("⏺ ", Style::new().fg(RED)),
                                Style::new().fg(RED),
                            ),
                        };
                        let mut spans = vec![
                            Span::raw(TASK_INDENT),
                            marker,
                            Span::styled(
                                task.agent.clone(),
                                Style::new().add_modifier(Modifier::BOLD | Modifier::DIM),
                            ),
                        ];
                        if !task.detail.is_empty() {
                            spans.push(Span::styled(format!("  {}", task.detail), detail_style));
                        }
                        out.push(Line::from(spans));
                    }
                }
                ChatEntry::Thinking(lines) => out.extend(lines.iter().cloned()),
                ChatEntry::Blank => out.push(Line::raw("")),
            }
        }
        out
    }
}

/// Indent nesting delegate task lines under their `⏺ Delegate(...)` marker.
const TASK_INDENT: &str = "    ";

/// Braille spinner frame, trailing space included so it drops in where a
/// settled `⏺ ` marker would go.
fn spinner(frame: u64) -> String {
    const BRAILLE: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    format!("{} ", BRAILLE[(frame as usize / 2) % BRAILLE.len()])
}

// ── Style mapping ────────────────────────────────────────────────
//
// We bypass termimad's `CompoundStyle` / crossterm `ContentStyle` entirely
// because termimad 0.34 re-exports crossterm 0.29 while our workspace uses
// crossterm 0.28 (required by ratatui 0.29).  Since WE define the MadSkin,
// we know exactly what colours map to which `CompositeKind`.

/// Base style for a line kind.  Must mirror the SKIN definition in render.rs.
fn kind_base_style(kind: CompositeKind) -> Style {
    match kind {
        CompositeKind::Header(1) => Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        CompositeKind::Header(2) => Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
        CompositeKind::Header(3) => Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
        CompositeKind::Header(_) => Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
        _ => Style::default(),
    }
}

/// Extra modifiers from compound-level markdown attributes.
fn compound_modifiers(compound: &termimad::minimad::Compound<'_>) -> Modifier {
    let mut m = Modifier::empty();
    if compound.bold {
        m |= Modifier::BOLD;
    }
    if compound.italic {
        m |= Modifier::ITALIC;
    }
    if compound.strikeout {
        m |= Modifier::CROSSED_OUT;
    }
    m
}

/// Left margin (in spaces) for a line kind.  Mirrors the SKIN definition.
fn kind_left_margin(kind: CompositeKind) -> usize {
    match kind {
        CompositeKind::Code => 4,
        _ => 2,
    }
}

// ── Markdown → ratatui Lines ─────────────────────────────────────

/// Render a markdown string through the given `MadSkin` (for wrapping and
/// structure) and convert to ratatui `Line` values.
pub fn markdown_to_lines(skin: &MadSkin, md: &str, width: usize) -> Vec<Line<'static>> {
    let fmt = FmtText::from(skin, md, Some(width));
    let mut out = Vec::with_capacity(fmt.lines.len());
    for fl in &fmt.lines {
        match fl {
            FmtLine::Normal(composite) => {
                let base = kind_base_style(composite.kind);
                let margin = kind_left_margin(composite.kind);
                let mut spans = Vec::new();
                if margin > 0 {
                    spans.push(Span::raw(" ".repeat(margin)));
                }
                for compound in &composite.compounds {
                    let extra = compound_modifiers(compound);
                    let style = if extra.is_empty() {
                        base
                    } else {
                        base.add_modifier(extra)
                    };
                    spans.push(Span::styled(compound.src.to_string(), style));
                }
                out.push(Line::from(spans));
            }
            FmtLine::TableRow(row) => {
                let mut spans = Vec::new();
                spans.push(Span::raw("  │"));
                for cell in &row.cells {
                    for compound in &cell.compounds {
                        spans.push(Span::raw(compound.src.to_string()));
                    }
                    spans.push(Span::raw("│"));
                }
                out.push(Line::from(spans));
            }
            FmtLine::TableRule(rule) => {
                let total: usize = rule.widths.iter().sum::<usize>() + rule.widths.len() + 1;
                out.push(Line::raw(format!("  {}", "─".repeat(total))));
            }
            FmtLine::HorizontalRule => {
                out.push(Line::raw(format!(
                    "  {}",
                    "─".repeat(width.saturating_sub(2))
                )));
            }
        }
    }
    out
}
