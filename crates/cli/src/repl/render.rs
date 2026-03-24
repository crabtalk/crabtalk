//! Streaming markdown renderer using termimad for text and syntect for code blocks.
//!
//! Output style matches Claude Code: `⏺` markers for text and tool calls,
//! `⎿` for tool results, 2-space continuation indent.

use console::{Style, Term, style};
use heck::ToUpperCamelCase;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::{
    io::{BufWriter, Stdout, Write},
    sync::LazyLock,
    time::Duration,
};
use syntect::{
    easy::HighlightLines, highlighting::ThemeSet, parsing::SyntaxSet,
    util::as_24_bit_terminal_escaped,
};
use termimad::MadSkin;

static S_DIM: LazyLock<Style> = LazyLock::new(|| Style::new().dim());
static S_BRAND: LazyLock<Style> = LazyLock::new(|| Style::new().color256(173));
static S_PROMPT: LazyLock<Style> = LazyLock::new(|| Style::new().color256(173).bold());
static S_BANNER: LazyLock<Style> = LazyLock::new(|| Style::new().color256(173).bold());
static S_GREEN: LazyLock<Style> = LazyLock::new(|| Style::new().color256(71));
static S_RED: LazyLock<Style> = LazyLock::new(|| Style::new().color256(204));
static S_SUBTLE: LazyLock<Style> = LazyLock::new(|| Style::new().color256(240));
static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

/// Skin with 2-space left margin (for text continuation after `⏺ `).
static SKIN: LazyLock<MadSkin> = LazyLock::new(|| {
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

/// Text continuation indent (aligns with text after `⏺ `).
const PAD: &str = "  ";
/// Tool result indent (aligns with label text after `⏺ `).
const TOOL_OUTPUT_MAX_SUCCESS: usize = 5;
const TOOL_OUTPUT_MAX_FAILURE: usize = 10;
const TOOL_PAD: &str = "  ";

const ERASE_LINE: &str = "\x1b[2K";

fn term_width() -> usize {
    Term::stdout().size().1 as usize
}

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
    out: BufWriter<Stdout>,
    /// Whether we've printed the first `⏺` marker for this response.
    started: bool,
    /// Whether the next text line is the very first (marker already on stdout).
    first_line: bool,
    /// Whether the last thing printed was a blank line (avoid double blanks).
    last_was_blank: bool,
    tool_labels: Vec<String>,
    tool_result_lines: usize,
    after_tool: bool,
    /// Whether any tool result in the current batch indicated failure.
    tool_failed: bool,
    /// Blinking spinner for waiting states (runs on background OS thread).
    spinner: Option<ProgressBar>,
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
            out: BufWriter::new(std::io::stdout()),
            started: false,
            first_line: false,
            last_was_blank: false,
            tool_labels: Vec::new(),
            tool_result_lines: 0,
            after_tool: false,
            tool_failed: false,
            spinner: None,
        }
    }

    /// Show a blinking dim `⏺` while waiting for content.
    /// Renders to stdout so all terminal writes are on one fd (no race).
    pub fn start_waiting(&mut self) {
        let _ = self.out.flush();
        self.clear_waiting();
        let sp = ProgressBar::with_draw_target(None, ProgressDrawTarget::stdout());
        sp.set_style(
            ProgressStyle::default_spinner()
                .tick_strings(&["⏺", " ", " "])
                .template("{spinner:.dim}")
                .expect("valid template"),
        );
        sp.enable_steady_tick(Duration::from_millis(500));
        sp.tick();
        self.spinner = Some(sp);
    }

    /// Stop the spinner and erase its line atomically.
    ///
    /// Holds the stdout lock while abandoning to prevent the spinner
    /// thread from writing one last frame after the erase.
    fn clear_waiting(&mut self) {
        if let Some(sp) = self.spinner.take() {
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            sp.abandon();
            let _ = write!(lock, "\r{ERASE_LINE}");
            let _ = lock.flush();
        }
    }

    /// Print the `⏺` marker at column 0 if this is the first text output.
    fn ensure_started(&mut self) {
        if !self.started {
            self.clear_waiting();
            let _ = write!(self.out, "⏺ ");
            self.started = true;
            self.first_line = true;
        }
    }

    pub fn push_text(&mut self, chunk: &str) {
        if chunk.is_empty() {
            return;
        }

        if matches!(self.state, RenderState::Thinking) {
            self.state = RenderState::Normal;
        }

        let was_started = self.started;
        self.ensure_started();

        // After a tool block, print a new `⏺` marker for continuing text.
        if self.after_tool {
            self.clear_waiting();
            // Only write ⏺ if ensure_started didn't just write one.
            if was_started {
                let _ = write!(self.out, "⏺ ");
            }
            self.first_line = true;
            self.after_tool = false;
        }

        for ch in chunk.chars() {
            if ch == '\n' {
                if self.first_line {
                    // Erase the raw-streamed text and re-render through
                    // termimad so markdown formatting (bold, etc.) applies.
                    let _ = write!(self.out, "\r{ERASE_LINE}⏺ ");
                    self.first_line = false;
                    let line = std::mem::take(&mut self.line_buf);
                    self.render_line(&line);
                } else {
                    let line = std::mem::take(&mut self.line_buf);
                    self.render_line(&line);
                }
            } else {
                self.line_buf.push(ch);
                // Stream first line chars inline for immediate visibility.
                if self.first_line {
                    let _ = write!(self.out, "{ch}");
                }
            }
        }

        let _ = self.out.flush();
    }

    pub fn push_thinking(&mut self, chunk: &str) {
        if chunk.is_empty() {
            return;
        }

        self.clear_waiting();

        if !matches!(self.state, RenderState::Thinking) {
            self.state = RenderState::Thinking;
        }
        let styled = style(chunk).dim().italic();
        let _ = write!(self.out, "{styled}");
        let _ = self.out.flush();
    }

    pub fn push_tool_start(&mut self, calls: &[(String, String)]) {
        // If markers already shown (early ToolCallsBegin), update labels
        // with full args and refresh the spinner message.
        if !self.tool_labels.is_empty() {
            self.tool_labels.clear();
            for (name, args) in calls {
                self.tool_labels.push(format_tool_label(name, args));
            }
            if let Some(ref sp) = self.spinner {
                sp.set_message(self.tool_labels.join(", "));
            }
            return;
        }
        self.flush_thinking();
        // Finalize any pending text BEFORE clear_waiting, so \r{ERASE_LINE}
        // doesn't eat inline text on the first line.
        if !self.line_buf.is_empty() {
            if self.first_line {
                let _ = writeln!(self.out);
                self.first_line = false;
                self.line_buf.clear();
            } else {
                let line = std::mem::take(&mut self.line_buf);
                self.render_md_line(&line);
            }
        }
        self.clear_waiting();
        if !self.last_was_blank {
            let _ = writeln!(self.out);
        }
        let _ = self.out.flush();

        self.tool_labels.clear();
        self.tool_failed = false;
        for (name, args) in calls {
            self.tool_labels.push(format_tool_label(name, args));
        }
        let _ = self.out.flush();

        // Show tool marker with braille spinner on stdout.
        let msg = self.tool_labels.join(", ");
        let sp = ProgressBar::with_draw_target(None, ProgressDrawTarget::stdout());
        sp.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.dim} {msg:.bold.dim}")
                .expect("valid template"),
        );
        sp.set_message(msg);
        sp.enable_steady_tick(Duration::from_millis(80));
        sp.tick();
        self.spinner = Some(sp);
    }

    /// Replace the spinner with static dim markers on stdout so
    /// push_tool_result / push_tool_done can use cursor movement.
    fn solidify_tool_markers(&mut self) {
        if self.spinner.is_some() {
            self.clear_waiting();
            for label in &self.tool_labels {
                let _ = writeln!(
                    self.out,
                    "{} {}",
                    S_BRAND.apply_to("⏺"),
                    style(label).bold().dim()
                );
            }
            let _ = self.out.flush();
        }
    }

    pub fn push_tool_result(&mut self, output: &str) {
        self.solidify_tool_markers();
        let failed = is_tool_failure(output);
        if failed {
            self.tool_failed = true;
        }
        let (lines, total) = format_tool_output(output, failed);
        let width = term_width().saturating_sub(TOOL_PAD.len() + 2);

        if lines.is_empty() {
            let _ = writeln!(
                self.out,
                "{TOOL_PAD}{} {}",
                S_SUBTLE.apply_to("⎿"),
                S_DIM.apply_to("(no output)")
            );
            self.tool_result_lines += 1;
        } else {
            for (i, line) in lines.iter().enumerate() {
                let truncated = if line.len() > width {
                    format!("{}...", &line[..width.saturating_sub(3)])
                } else {
                    line.clone()
                };
                if i == 0 {
                    let _ = writeln!(
                        self.out,
                        "{TOOL_PAD}{} {}",
                        S_SUBTLE.apply_to("⎿"),
                        S_DIM.apply_to(&truncated)
                    );
                } else {
                    let _ = writeln!(self.out, "{TOOL_PAD}  {}", S_DIM.apply_to(&truncated));
                }
                self.tool_result_lines += 1;
            }
            let shown = lines.len();
            if total > shown {
                let _ = writeln!(
                    self.out,
                    "{TOOL_PAD}  {}",
                    S_DIM.apply_to(format!("… +{} lines", total - shown))
                );
                self.tool_result_lines += 1;
            }
        }

        let _ = writeln!(self.out);
        self.tool_result_lines += 1;
        let _ = self.out.flush();
    }

    pub fn push_tool_done(&mut self, _success: bool) {
        self.solidify_tool_markers();
        let count = self.tool_labels.len();
        if count == 0 {
            return;
        }

        let success = !self.tool_failed;
        let _ = self.out.flush();
        let marker = if success {
            S_GREEN.apply_to("⏺")
        } else {
            S_RED.apply_to("⏺")
        };

        let up = count + self.tool_result_lines;
        let _ = write!(self.out, "\x1b[{up}A");
        for label in &self.tool_labels {
            let _ = write!(
                self.out,
                "\r{ERASE_LINE}{marker} {}\n",
                style(label).bold().dim()
            );
        }
        if self.tool_result_lines > 0 {
            let _ = write!(self.out, "\x1b[{}B", self.tool_result_lines);
        }
        let _ = self.out.flush();
        self.tool_labels.clear();
        self.tool_result_lines = 0;
        self.after_tool = true;
    }

    pub fn finish(&mut self) {
        if !self.tool_labels.is_empty() {
            // push_tool_done → solidify_tool_markers handles the spinner.
            self.push_tool_done(false);
        } else {
            self.clear_waiting();
        }
        self.flush_thinking();

        if !self.line_buf.is_empty() {
            if self.first_line {
                // First line was streamed inline — just finalize.
                let _ = writeln!(self.out);
                self.line_buf.clear();
            } else {
                let line = std::mem::take(&mut self.line_buf);
                match &self.state {
                    RenderState::CodeBlock { .. } => {
                        self.flush_code_block_raw(&line);
                    }
                    _ => {
                        self.ensure_started();
                        self.render_md_line(&line);
                    }
                }
            }
        }

        if matches!(self.state, RenderState::Table(_)) {
            self.flush_table();
        }

        if let RenderState::CodeBlock { lang, code } = &self.state {
            let lang = lang.clone();
            let code = code.clone();
            self.emit_code_block(&lang, &code);
            self.state = RenderState::Normal;
        }

        let _ = self.out.flush();
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
                    self.print_code_border_top(&lang);
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
            let buf = buf.clone();
            let width = term_width().saturating_sub(PAD.len());
            let rendered = format!("{}", SKIN.text(&buf, Some(width)));
            for line in rendered.lines() {
                let _ = writeln!(self.out, "{PAD}{line}");
            }
        }
        self.state = RenderState::Normal;
    }

    /// Render a single markdown line through termimad.
    fn render_md_line(&mut self, line: &str) {
        if line.is_empty() {
            let _ = writeln!(self.out);
            self.last_was_blank = true;
            self.first_line = false;
            return;
        }
        self.last_was_blank = false;

        let width = term_width();
        let text = format!("{}", SKIN.text(line, Some(width)));
        // SKIN adds a 2-char left margin. On the first line, `⏺ ` is
        // already on stdout, so strip the margin to avoid double-indent.
        if self.first_line {
            self.first_line = false;
            let _ = write!(self.out, "{}", text.trim_start());
            return;
        }
        let _ = write!(self.out, "{text}");
    }

    fn print_code_border_top(&mut self, lang: &str) {
        self.first_line = false;
        let label = if lang.is_empty() {
            "┌─".to_string()
        } else {
            format!("┌ {lang} ─")
        };
        let _ = writeln!(self.out, "{PAD}{}", S_SUBTLE.apply_to(label));
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
        let pipe = S_SUBTLE.apply_to("│");

        for line in code.lines() {
            match h.highlight_line(line, &SYNTAX_SET) {
                Ok(ranges) => {
                    let escaped = as_24_bit_terminal_escaped(&ranges[..], false);
                    let _ = writeln!(self.out, "{PAD}{pipe} {escaped}\x1b[0m");
                }
                Err(_) => {
                    let _ = writeln!(self.out, "{PAD}{pipe} {line}");
                }
            }
        }

        let _ = writeln!(self.out, "{PAD}{}", S_SUBTLE.apply_to("└─"));
    }

    fn flush_code_block_raw(&mut self, extra: &str) {
        let pipe = S_SUBTLE.apply_to("│");
        if let RenderState::CodeBlock { code, .. } = &self.state {
            let full = format!("{code}{extra}");
            for line in full.lines() {
                let _ = writeln!(self.out, "{PAD}{pipe} {line}");
            }
        }
        let _ = writeln!(self.out, "{PAD}{}", S_SUBTLE.apply_to("└─"));
        self.state = RenderState::Normal;
    }

    fn flush_thinking(&mut self) {
        if matches!(self.state, RenderState::Thinking) {
            self.state = RenderState::Normal;
        }
    }
}

pub fn styled_prompt(agent: &str) -> String {
    format!("{} > ", S_PROMPT.apply_to(agent))
}

pub fn welcome_banner(model: Option<&str>) -> String {
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
        S_BANNER.apply_to(&title),
        S_DIM.apply_to(format!("  {rule}")),
    )
}

/// Check if tool output indicates failure.
fn is_tool_failure(output: &str) -> bool {
    // Bash JSON result with non-zero exit code.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(output)
        && let Some(code) = v.get("exit_code").and_then(|c| c.as_i64())
    {
        return code != 0;
    }
    // Generic failure patterns.
    output.starts_with("bash failed:")
        || output.starts_with("permission denied:")
        || output.starts_with("tool not available:")
        || output.starts_with("invalid arguments:")
}

/// Extract output lines from a tool result, truncated to `max` lines.
/// Returns (lines, total_count) where total_count is the full line count.
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

fn format_tool_label(name: &str, args: &str) -> String {
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

    // Show only the first line — multi-line commands bloat the label.
    let first_line = cmd.lines().next().unwrap_or(cmd);
    let max = term_width().saturating_sub(8);
    let display = if first_line.len() > max {
        format!("{}...", &first_line[..max])
    } else if first_line.len() < cmd.len() {
        format!("{first_line}...")
    } else {
        first_line.to_string()
    };
    format!("Bash({display})")
}
