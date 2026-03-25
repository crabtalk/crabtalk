//! Streaming markdown renderer producing ratatui `Line` values.
//!
//! Output style matches Claude Code: `⏺` markers for text and tool calls,
//! `⎿` for tool results, 2-space continuation indent.
//!
//! All output goes into a [`ChatBuffer`] — nothing is written to stdout.

use crate::repl::chat::{
    ChatBuffer, ChatEntry, S_DIM, S_SUBTLE, SUBTLE, markdown_to_lines, syntect_to_ratatui,
};
use console::style;
use heck::ToUpperCamelCase;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::sync::LazyLock;
use syntect::{easy::HighlightLines, highlighting::ThemeSet, parsing::SyntaxSet};
use termimad::MadSkin;

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

/// Text continuation indent (aligns with text after `⏺ `).
const PAD: &str = "  ";
const TOOL_OUTPUT_MAX_SUCCESS: usize = 5;
const TOOL_OUTPUT_MAX_FAILURE: usize = 10;
const TOOL_PAD: &str = "  ";

/// Skin with 2-space left margin (for text continuation after `⏺ `).
pub static SKIN: LazyLock<MadSkin> = LazyLock::new(|| {
    use termimad::crossterm::style::{Attribute, Color};
    let mut skin = MadSkin::default_dark();
    skin.paragraph.left_margin = 2;
    skin.headers[0]
        .compound_style
        .set_fgbg(Color::Cyan, Color::Reset);
    skin.headers[0].compound_style.add_attr(Attribute::Bold);
    skin.headers[0].left_margin = 2;
    skin.headers[1]
        .compound_style
        .set_fgbg(Color::Magenta, Color::Reset);
    skin.headers[1].compound_style.add_attr(Attribute::Bold);
    skin.headers[1].left_margin = 2;
    skin.headers[2]
        .compound_style
        .set_fgbg(Color::White, Color::Reset);
    skin.headers[2].compound_style.add_attr(Attribute::Bold);
    skin.headers[2].left_margin = 2;
    skin.code_block.left_margin = 4;
    skin
});

#[derive(Default)]
enum RenderState {
    #[default]
    Normal,
    CodeBlock {
        lang: String,
        code: String,
    },
    Table(String),
    Thinking,
}

pub struct MarkdownRenderer {
    line_buf: String,
    state: RenderState,
    pub buffer: ChatBuffer,
    width: usize,
    /// Whether we've emitted the first `⏺` marker for this response.
    started: bool,
    /// Whether the next text line is the very first in the response.
    first_line: bool,
    /// Whether the last thing emitted was a blank line (avoid double blanks).
    last_was_blank: bool,
    tool_labels: Vec<String>,
    after_tool: bool,
    tool_failed: bool,
    /// True while waiting for content (drives spinner in the UI).
    pub waiting: bool,
    /// Accumulator for thinking text (emitted as one entry when switching back).
    thinking_buf: String,
}

impl Default for MarkdownRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkdownRenderer {
    pub fn new() -> Self {
        Self {
            line_buf: String::new(),
            state: RenderState::Normal,
            buffer: ChatBuffer::new(),
            width: 80,
            started: false,
            first_line: false,
            last_was_blank: false,
            tool_labels: Vec::new(),
            after_tool: false,
            tool_failed: false,
            waiting: false,
            thinking_buf: String::new(),
        }
    }

    pub fn set_width(&mut self, width: usize) {
        self.width = width;
    }

    /// Signal that we're waiting for a new response.
    ///
    /// Resets streaming state so the first text of the new response gets a
    /// fresh `⏺` marker.  Without this, `started`/`first_line` from the
    /// previous response would bleed through and suppress the marker.
    pub fn start_waiting(&mut self) {
        self.waiting = true;
        self.started = false;
        self.first_line = false;
        self.after_tool = false;
        self.tool_labels.clear();
        self.tool_failed = false;
    }

    /// The partially-streamed current line (displayed at the bottom of chat).
    pub fn current_line(&self) -> Option<Line<'static>> {
        if self.line_buf.is_empty() {
            return None;
        }
        if self.first_line || !self.started {
            // First line gets the ⏺ marker prefix.
            Some(Line::from(vec![
                Span::styled("⏺ ", Style::new().fg(Color::Indexed(173))),
                Span::raw(self.line_buf.clone()),
            ]))
        } else {
            Some(Line::from(vec![
                Span::raw(PAD.to_string()),
                Span::raw(self.line_buf.clone()),
            ]))
        }
    }

    pub fn push_text(&mut self, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        self.waiting = false;

        if matches!(self.state, RenderState::Thinking) {
            self.flush_thinking();
        }

        self.ensure_started();

        if self.after_tool {
            self.after_tool = false;
            self.first_line = true;
        }

        for ch in chunk.chars() {
            if ch == '\n' {
                let line = std::mem::take(&mut self.line_buf);
                self.render_line(&line);
                self.first_line = false;
            } else {
                self.line_buf.push(ch);
            }
        }
    }

    pub fn push_thinking(&mut self, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        self.waiting = false;

        if !matches!(self.state, RenderState::Thinking) {
            self.state = RenderState::Thinking;
        }
        self.thinking_buf.push_str(chunk);
    }

    pub fn push_tool_start(&mut self, calls: &[(String, String)]) {
        // If markers already shown (early ToolCallsBegin), update labels.
        if !self.tool_labels.is_empty() {
            self.tool_labels.clear();
            for (name, args) in calls {
                self.tool_labels
                    .push(format_tool_label(name, args, self.width));
            }
            // Update the existing ToolMarker entry in the buffer.
            for entry in self.buffer.entries_mut().iter_mut().rev() {
                if let ChatEntry::ToolMarker { labels, .. } = entry {
                    labels.clone_from(&self.tool_labels);
                    break;
                }
            }
            return;
        }

        self.flush_thinking();
        self.finalize_line_buf();
        self.waiting = false;

        if !self.last_was_blank {
            self.buffer.push(ChatEntry::Blank);
        }

        self.tool_labels.clear();
        self.tool_failed = false;
        for (name, args) in calls {
            self.tool_labels
                .push(format_tool_label(name, args, self.width));
        }

        self.buffer.push(ChatEntry::ToolMarker {
            labels: self.tool_labels.clone(),
            status: crate::repl::chat::ToolStatus::Running,
        });
    }

    pub fn push_tool_result(&mut self, output: &str) {
        let failed = is_tool_failure(output);
        if failed {
            self.tool_failed = true;
        }
        let (text_lines, total) = format_tool_output(output, failed);
        let max_width = self.width.saturating_sub(TOOL_PAD.len() + 2);

        let mut lines = Vec::new();

        if text_lines.is_empty() {
            lines.push(Line::from(vec![
                Span::raw(TOOL_PAD.to_string()),
                Span::styled("⎿ ", S_SUBTLE),
                Span::styled("(no output)", S_DIM),
            ]));
        } else {
            for (i, line) in text_lines.iter().enumerate() {
                let truncated = if line.len() > max_width {
                    format!("{}...", &line[..max_width.saturating_sub(3)])
                } else {
                    line.clone()
                };
                if i == 0 {
                    lines.push(Line::from(vec![
                        Span::raw(TOOL_PAD.to_string()),
                        Span::styled("⎿ ", S_SUBTLE),
                        Span::styled(truncated, S_DIM),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::raw(format!("{TOOL_PAD}  ")),
                        Span::styled(truncated, S_DIM),
                    ]));
                }
            }
            let shown = text_lines.len();
            if total > shown {
                lines.push(Line::from(vec![
                    Span::raw(format!("{TOOL_PAD}  ")),
                    Span::styled(format!("… +{} lines", total - shown), S_DIM),
                ]));
            }
        }
        lines.push(Line::raw(""));

        self.buffer.push(ChatEntry::ToolResult(lines));
    }

    pub fn push_tool_done(&mut self, _success: bool) {
        let success = !self.tool_failed;
        self.buffer.finish_tool(success);
        self.tool_labels.clear();
        self.after_tool = true;
    }

    pub fn finish(&mut self) {
        if !self.tool_labels.is_empty() {
            self.push_tool_done(false);
        }
        self.flush_thinking();
        self.finalize_line_buf();
        self.waiting = false;
    }

    // ── Internal ─────────────────────────────────────────────────

    fn ensure_started(&mut self) {
        if !self.started {
            self.waiting = false;
            self.started = true;
            self.first_line = true;
        }
    }

    /// Finalize any partial line_buf content into the buffer.
    fn finalize_line_buf(&mut self) {
        if self.line_buf.is_empty() {
            return;
        }
        let line = std::mem::take(&mut self.line_buf);
        match &self.state {
            RenderState::CodeBlock { .. } => {
                self.flush_code_block_raw(&line);
            }
            _ => {
                // Use ⏺ marker on the first line, PAD otherwise.
                let prefix = if self.first_line {
                    Span::styled("⏺ ", Style::new().fg(Color::Indexed(173)))
                } else {
                    Span::raw(PAD.to_string())
                };
                self.buffer.push(ChatEntry::Text(vec![Line::from(vec![
                    prefix,
                    Span::raw(line),
                ])]));
            }
        }
        self.first_line = false;
    }

    fn render_line(&mut self, line: &str) {
        match &mut self.state {
            RenderState::CodeBlock { lang: _, code } => {
                if line.starts_with("```") {
                    let lang = if let RenderState::CodeBlock { lang, .. } = &self.state {
                        lang.clone()
                    } else {
                        String::new()
                    };
                    let code = if let RenderState::CodeBlock { code, .. } = &self.state {
                        code.clone()
                    } else {
                        String::new()
                    };
                    self.emit_code_block(&lang, &code);
                    self.state = RenderState::Normal;
                } else {
                    code.push_str(line);
                    code.push('\n');
                }
            }
            RenderState::Table(buf) => {
                if line.starts_with('|') || line.starts_with("|-") {
                    buf.push_str(line);
                    buf.push('\n');
                } else {
                    self.flush_table();
                    self.render_line(line);
                }
            }
            RenderState::Normal | RenderState::Thinking => {
                if let Some(rest) = line.strip_prefix("```") {
                    let lang = rest.trim().to_string();
                    self.emit_code_border_top(&lang);
                    self.state = RenderState::CodeBlock {
                        lang,
                        code: String::new(),
                    };
                } else if line.starts_with('|') {
                    let mut buf = String::new();
                    buf.push_str(line);
                    buf.push('\n');
                    self.state = RenderState::Table(buf);
                } else {
                    self.render_md_line(line);
                }
            }
        }
    }

    fn flush_table(&mut self) {
        if let RenderState::Table(buf) = &self.state {
            let lines = markdown_to_lines(&SKIN, buf, self.width.saturating_sub(PAD.len()));
            self.buffer.push(ChatEntry::Text(lines));
        }
        self.state = RenderState::Normal;
    }

    fn render_md_line(&mut self, line: &str) {
        if line.is_empty() {
            self.buffer.push(ChatEntry::Blank);
            self.last_was_blank = true;
            return;
        }
        self.last_was_blank = false;

        let mut lines = markdown_to_lines(&SKIN, line, self.width);

        // On the first line, prepend the ⏺ marker.
        if self.first_line && !lines.is_empty() {
            let first = lines.remove(0);
            // Strip the skin's left margin (we replace it with ⏺ marker).
            let spans: Vec<Span> = first
                .spans
                .into_iter()
                .skip_while(|s| s.content.chars().all(|c| c == ' '))
                .collect();
            let mut new_spans = vec![Span::styled("⏺ ", Style::new().fg(Color::Indexed(173)))];
            new_spans.extend(spans);
            lines.insert(0, Line::from(new_spans));
        }

        self.buffer.push(ChatEntry::Text(lines));
    }

    fn emit_code_border_top(&mut self, lang: &str) {
        self.first_line = false;
        let label = if lang.is_empty() {
            "┌─".to_string()
        } else {
            format!("┌ {lang} ─")
        };
        self.buffer.push(ChatEntry::Text(vec![Line::from(vec![
            Span::raw(PAD.to_string()),
            Span::styled(label, S_SUBTLE),
        ])]));
    }

    fn emit_code_block(&mut self, lang: &str, code: &str) {
        let syntax = if lang.is_empty() {
            SYNTAX_SET.find_syntax_plain_text()
        } else {
            SYNTAX_SET
                .find_syntax_by_token(lang)
                .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text())
        };

        let theme = &THEME_SET.themes["base16-ocean.dark"];
        let mut h = HighlightLines::new(syntax, theme);
        let pipe_style = Style::new().fg(SUBTLE);

        let mut lines = Vec::new();
        for line in code.lines() {
            match h.highlight_line(line, &SYNTAX_SET) {
                Ok(ranges) => {
                    let mut spans =
                        vec![Span::raw(PAD.to_string()), Span::styled("│ ", pipe_style)];
                    for (ss, text) in &ranges {
                        spans.push(Span::styled(text.to_string(), syntect_to_ratatui(ss)));
                    }
                    lines.push(Line::from(spans));
                }
                Err(_) => {
                    lines.push(Line::from(vec![
                        Span::raw(PAD.to_string()),
                        Span::styled("│ ", pipe_style),
                        Span::raw(line.to_string()),
                    ]));
                }
            }
        }
        lines.push(Line::from(vec![
            Span::raw(PAD.to_string()),
            Span::styled("└─", S_SUBTLE),
        ]));
        self.buffer.push(ChatEntry::Text(lines));
    }

    fn flush_code_block_raw(&mut self, extra: &str) {
        if let RenderState::CodeBlock { code, .. } = &self.state {
            let full = format!("{code}{extra}");
            let pipe_style = Style::new().fg(SUBTLE);
            let mut lines = Vec::new();
            for line in full.lines() {
                lines.push(Line::from(vec![
                    Span::raw(PAD.to_string()),
                    Span::styled("│ ", pipe_style),
                    Span::raw(line.to_string()),
                ]));
            }
            lines.push(Line::from(vec![
                Span::raw(PAD.to_string()),
                Span::styled("└─", S_SUBTLE),
            ]));
            self.buffer.push(ChatEntry::Text(lines));
        }
        self.state = RenderState::Normal;
    }

    fn flush_thinking(&mut self) {
        if !self.thinking_buf.is_empty() {
            let text = std::mem::take(&mut self.thinking_buf);
            let thinking_style = Style::new().add_modifier(Modifier::DIM | Modifier::ITALIC);
            let lines: Vec<Line<'static>> = text
                .lines()
                .map(|l| Line::from(Span::styled(l.to_string(), thinking_style)))
                .collect();
            if !lines.is_empty() {
                self.buffer.push(ChatEntry::Thinking(lines));
            }
        }
        if matches!(self.state, RenderState::Thinking) {
            self.state = RenderState::Normal;
        }
    }
}

// ── Public helpers (used by REPL and other modules) ──────────────

pub fn styled_prompt(agent: &str) -> String {
    let s = console::Style::new().color256(173).bold();
    format!("{} > ", s.apply_to(agent))
}

pub fn welcome_banner(model: Option<&str>) -> String {
    let s_banner = console::Style::new().color256(173).bold();
    let s_dim = console::Style::new().dim();
    let model_part = match model {
        Some(m) => format!(" ({m})"),
        None => String::new(),
    };
    let title = format!("  Crabtalk{model_part} — Ctrl+D to exit");
    let width = title.len().min(60);
    let rule: String = "─".repeat(width);
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "?".to_string());
    let cwd_line = style(format!("  ~ {cwd}")).bold().dim();
    format!(
        "{}\n{}\n{cwd_line}",
        s_banner.apply_to(&title),
        s_dim.apply_to(format!("  {rule}")),
    )
}

// ── Tool output helpers ──────────────────────────────────────────

/// Check if tool output indicates failure.
fn is_tool_failure(output: &str) -> bool {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(output)
        && let Some(code) = v.get("exit_code").and_then(|c| c.as_i64())
    {
        return code != 0;
    }
    output.starts_with("bash failed:")
        || output.starts_with("permission denied:")
        || output.starts_with("tool not available:")
        || output.starts_with("invalid arguments:")
}

fn format_tool_output(output: &str, failed: bool) -> (Vec<String>, usize) {
    let max = if failed {
        TOOL_OUTPUT_MAX_FAILURE
    } else {
        TOOL_OUTPUT_MAX_SUCCESS
    };

    if let Ok(v) = serde_json::from_str::<serde_json::Value>(output)
        && let (Some(stdout), Some(stderr), Some(exit_code)) = (
            v.get("stdout").and_then(|s| s.as_str()),
            v.get("stderr").and_then(|s| s.as_str()),
            v.get("exit_code").and_then(|c| c.as_i64()),
        )
    {
        let text = if exit_code != 0 {
            if stderr.is_empty() { stdout } else { stderr }
        } else if stdout.is_empty() {
            stderr
        } else {
            stdout
        };
        return truncate_lines(text, max);
    }

    truncate_lines(output, max)
}

fn truncate_lines(text: &str, max: usize) -> (Vec<String>, usize) {
    let all: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    let total = all.len();
    let lines = all.into_iter().take(max).map(String::from).collect();
    (lines, total)
}

fn format_tool_label(name: &str, args: &str, width: usize) -> String {
    let pascal = name.to_upper_camel_case();

    if name != "bash" {
        return pascal;
    }

    let Ok(v) = serde_json::from_str::<serde_json::Value>(args) else {
        return pascal;
    };

    let Some(cmd) = v.get("command").and_then(|c| c.as_str()) else {
        return pascal;
    };

    let first_line = cmd.lines().next().unwrap_or(cmd);
    let max = width.saturating_sub(8);
    let display = if first_line.len() > max {
        format!("{}...", &first_line[..max])
    } else if first_line.len() < cmd.len() {
        format!("{first_line}...")
    } else {
        first_line.to_string()
    };
    format!("Bash({display})")
}
