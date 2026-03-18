//! Streaming markdown renderer with syntax-highlighted code blocks.

use console::{Style, Term, style};
use std::{
    io::{BufWriter, Stdout, Write},
    sync::LazyLock,
};
use syntect::{
    easy::HighlightLines, highlighting::ThemeSet, parsing::SyntaxSet,
    util::as_24_bit_terminal_escaped,
};

// Reusable styles.
static S_BOLD: LazyLock<Style> = LazyLock::new(|| Style::new().bold());
static S_DIM: LazyLock<Style> = LazyLock::new(|| Style::new().dim());
static S_ITALIC: LazyLock<Style> = LazyLock::new(|| Style::new().italic());
static S_H1: LazyLock<Style> = LazyLock::new(|| Style::new().bold().cyan().bright());
static S_H2: LazyLock<Style> = LazyLock::new(|| Style::new().bold().blue().bright());
static S_H3: LazyLock<Style> = LazyLock::new(|| Style::new().bold().white().bright());
static S_CODE: LazyLock<Style> = LazyLock::new(|| Style::new().cyan());
static S_PROMPT: LazyLock<Style> = LazyLock::new(|| Style::new().bold().green().bright());
static S_BANNER: LazyLock<Style> = LazyLock::new(|| Style::new().bold().yellow().bright());

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

/// Terminal width with fallback.
fn term_width() -> usize {
    Term::stdout().size().1 as usize
}

/// Renderer state tracking whether we're in normal text, a code block, or thinking.
#[derive(Default)]
enum RenderState {
    #[default]
    Normal,
    CodeBlock {
        lang: String,
        code: String,
    },
    Thinking,
}

/// Streaming markdown renderer that buffers lines and emits styled output.
pub struct MarkdownRenderer {
    line_buf: String,
    state: RenderState,
    out: BufWriter<Stdout>,
}

impl Default for MarkdownRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkdownRenderer {
    /// Create a new renderer.
    pub fn new() -> Self {
        Self {
            line_buf: String::new(),
            state: RenderState::Normal,
            out: BufWriter::new(std::io::stdout()),
        }
    }

    /// Push streamed text content, rendering complete lines.
    pub fn push_text(&mut self, chunk: &str) {
        if matches!(self.state, RenderState::Thinking) {
            self.state = RenderState::Normal;
        }

        for ch in chunk.chars() {
            if ch == '\n' {
                let line = std::mem::take(&mut self.line_buf);
                self.render_line(&line);
            } else {
                self.line_buf.push(ch);
            }
        }
        let _ = self.out.flush();
    }

    /// Push thinking/reasoning content (dim + italic).
    pub fn push_thinking(&mut self, chunk: &str) {
        if !matches!(self.state, RenderState::Thinking) {
            self.state = RenderState::Thinking;
        }
        let styled = style(chunk).dim().italic();
        let _ = write!(self.out, "{styled}");
        let _ = self.out.flush();
    }

    /// Print tool call names as a dim line.
    pub fn push_tool_start(&mut self, names: &[String]) {
        self.flush_thinking();
        let label = names.join(", ");
        let _ = writeln!(self.out, "{}", S_DIM.apply_to(format!("  [{label}]")));
        let _ = self.out.flush();
    }

    /// No-op — tool start already printed a full line.
    pub fn push_tool_done(&mut self) {}

    /// Flush remaining buffer on stream end.
    pub fn finish(&mut self) {
        self.flush_thinking();

        // Flush remaining line buffer.
        if !self.line_buf.is_empty() {
            let line = std::mem::take(&mut self.line_buf);
            match &self.state {
                RenderState::CodeBlock { .. } => {
                    self.flush_code_block_raw(&line);
                }
                _ => {
                    self.render_inline(&line);
                    let _ = writeln!(self.out);
                }
            }
        }

        // If still in a code block, flush it.
        if let RenderState::CodeBlock { lang, code } = &self.state {
            let lang = lang.clone();
            let code = code.clone();
            self.emit_code_block(&lang, &code);
            self.state = RenderState::Normal;
        }

        let _ = self.out.flush();
    }

    /// Render a complete line based on current state.
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
            RenderState::Normal | RenderState::Thinking => {
                if let Some(rest) = line.strip_prefix("```") {
                    let lang = rest.trim().to_string();
                    self.print_code_border_top(&lang);
                    self.state = RenderState::CodeBlock {
                        lang,
                        code: String::new(),
                    };
                } else if let Some(rest) = line.strip_prefix("### ") {
                    let _ = writeln!(self.out, "{}", S_H3.apply_to(rest));
                } else if let Some(rest) = line.strip_prefix("## ") {
                    let _ = writeln!(self.out, "{}", S_H2.apply_to(rest));
                } else if let Some(rest) = line.strip_prefix("# ") {
                    let _ = writeln!(self.out, "{}", S_H1.apply_to(rest));
                } else if line == "---" || line == "***" || line == "___" {
                    let w = term_width().min(60);
                    let rule: String = "─".repeat(w);
                    let _ = writeln!(self.out, "{}", S_DIM.apply_to(rule));
                } else if let Some(rest) =
                    line.strip_prefix("- ").or_else(|| line.strip_prefix("* "))
                {
                    let _ = write!(self.out, "  • ");
                    self.render_inline(rest);
                    let _ = writeln!(self.out);
                } else if is_ordered_list(line) {
                    let (prefix, rest) = split_ordered_list(line);
                    let _ = write!(self.out, "  {prefix}");
                    self.render_inline(rest);
                    let _ = writeln!(self.out);
                } else {
                    self.render_inline(line);
                    let _ = writeln!(self.out);
                }
            }
        }
    }

    /// Render inline markdown formatting: **bold**, *italic*, `code`.
    fn render_inline(&mut self, text: &str) {
        let chars: Vec<char> = text.chars().collect();
        let len = chars.len();
        let mut i = 0;

        while i < len {
            if i + 1 < len
                && chars[i] == '*'
                && chars[i + 1] == '*'
                && let Some(end) = find_closing(&chars, i + 2, "**")
            {
                let inner: String = chars[i + 2..end].iter().collect();
                let _ = write!(self.out, "{}", S_BOLD.apply_to(&inner));
                i = end + 2;
                continue;
            }
            if chars[i] == '*'
                && (i + 1 >= len || chars[i + 1] != '*')
                && let Some(end) = find_closing_char(&chars, i + 1, '*')
            {
                let inner: String = chars[i + 1..end].iter().collect();
                let _ = write!(self.out, "{}", S_ITALIC.apply_to(&inner));
                i = end + 1;
                continue;
            }
            if chars[i] == '`'
                && let Some(end) = find_closing_char(&chars, i + 1, '`')
            {
                let inner: String = chars[i + 1..end].iter().collect();
                let _ = write!(self.out, "{}", S_CODE.apply_to(&inner));
                i = end + 1;
                continue;
            }
            let _ = write!(self.out, "{}", chars[i]);
            i += 1;
        }
    }

    /// Print the top border of a code block with optional language label.
    fn print_code_border_top(&mut self, lang: &str) {
        let label = if lang.is_empty() {
            "┌─".to_string()
        } else {
            format!("┌ {lang} ─")
        };
        let _ = writeln!(self.out, "{}", S_DIM.apply_to(label));
    }

    /// Syntax-highlight and emit a buffered code block.
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
        let pipe = S_DIM.apply_to("│");

        for line in code.lines() {
            match h.highlight_line(line, &SYNTAX_SET) {
                Ok(ranges) => {
                    let escaped = as_24_bit_terminal_escaped(&ranges[..], false);
                    let _ = writeln!(self.out, "{pipe} {escaped}\x1b[0m");
                }
                Err(_) => {
                    let _ = writeln!(self.out, "{pipe} {line}");
                }
            }
        }

        let _ = writeln!(self.out, "{}", S_DIM.apply_to("└─"));
    }

    /// Flush a code block as raw text (used on cancel).
    fn flush_code_block_raw(&mut self, extra: &str) {
        let pipe = S_DIM.apply_to("│");
        if let RenderState::CodeBlock { code, .. } = &self.state {
            let full = format!("{code}{extra}");
            for line in full.lines() {
                let _ = writeln!(self.out, "{pipe} {line}");
            }
        }
        let _ = writeln!(self.out, "{}", S_DIM.apply_to("└─"));
        self.state = RenderState::Normal;
    }

    /// If in thinking state, transition back to normal.
    fn flush_thinking(&mut self) {
        if matches!(self.state, RenderState::Thinking) {
            self.state = RenderState::Normal;
        }
    }
}

/// Build the styled prompt string for rustyline.
pub fn styled_prompt(agent: &str) -> String {
    format!("{} > ", S_PROMPT.apply_to(agent))
}

/// Build a welcome banner with optional model name.
pub fn welcome_banner(model: Option<&str>) -> String {
    let model_part = match model {
        Some(m) => format!(" ({m})"),
        None => String::new(),
    };
    let title = format!("  Walrus{model_part} — Ctrl+D to exit");
    let width = title.len().min(60);
    let rule: String = "─".repeat(width);
    format!(
        "{}\n{}",
        S_BANNER.apply_to(&title),
        S_DIM.apply_to(format!("  {rule}"))
    )
}

/// Check if a line is an ordered list item (e.g. "1. foo").
fn is_ordered_list(line: &str) -> bool {
    let mut chars = line.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_digit()) {
        return false;
    }
    for c in chars {
        if c == '.' {
            return true;
        }
        if !c.is_ascii_digit() {
            return false;
        }
    }
    false
}

/// Split an ordered list item into prefix ("1. ") and rest.
fn split_ordered_list(line: &str) -> (&str, &str) {
    if let Some(dot) = line.find(". ") {
        (&line[..dot + 2], &line[dot + 2..])
    } else {
        (line, "")
    }
}

/// Find closing `**` in chars starting at `start`.
fn find_closing(chars: &[char], start: usize, pattern: &str) -> Option<usize> {
    let pat: Vec<char> = pattern.chars().collect();
    let plen = pat.len();
    for i in start..chars.len().saturating_sub(plen - 1) {
        if chars[i..i + plen] == pat[..] {
            return Some(i);
        }
    }
    None
}

/// Find a single closing character in chars starting at `start`.
fn find_closing_char(chars: &[char], start: usize, closing: char) -> Option<usize> {
    (start..chars.len()).find(|&i| chars[i] == closing)
}
