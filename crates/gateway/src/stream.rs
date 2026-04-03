//! Platform-agnostic stream accumulator for gateway loops.
//!
//! Consumes `StreamEvent` messages from the daemon and builds a text buffer
//! with inline tool call status. Used by the Telegram loop.

use wcore::protocol::message::{AskQuestion, StreamEvent, stream_event};

/// Accumulates streaming events into a renderable text buffer.
pub struct StreamAccumulator {
    /// Accumulated response text.
    text: String,
    /// Current tool call status line (e.g., "[calling bash, read...]").
    tool_line: Option<String>,
    /// Conversation ID from StreamStart.
    pub session: Option<u64>,
    /// Captured error, if any.
    error: Option<String>,
    /// Whether the stream has ended.
    pub done: bool,
    /// Pending structured questions from an `AskUserEvent`.
    pending_questions: Option<Vec<AskQuestion>>,
}

impl Default for StreamAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamAccumulator {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            tool_line: None,
            session: None,
            error: None,
            done: false,
            pending_questions: None,
        }
    }

    /// Push a stream event into the accumulator.
    pub fn push(&mut self, event: &StreamEvent) {
        match &event.event {
            Some(stream_event::Event::Start(s)) => {
                self.session = Some(s.session);
            }
            Some(stream_event::Event::Chunk(c)) => {
                self.text.push_str(&c.content);
            }
            Some(stream_event::Event::Thinking(_)) => {
                // Thinking content not shown in gateway messages.
            }
            Some(stream_event::Event::ToolStart(ts)) => {
                let names: Vec<&str> = ts.calls.iter().map(|c| c.name.as_str()).collect();
                self.tool_line = Some(format!("[calling {}...]", names.join(", ")));
            }
            Some(stream_event::Event::ToolResult(_)) => {}
            Some(stream_event::Event::ToolsComplete(_)) => {
                self.tool_line = None;
            }
            Some(stream_event::Event::End(end)) => {
                if !end.error.is_empty() {
                    self.set_error(end.error.clone());
                }
                self.done = true;
            }
            Some(stream_event::Event::AskUser(ask)) => {
                let headers: Vec<&str> = ask.questions.iter().map(|q| q.header.as_str()).collect();
                self.tool_line = Some(format!("[question: {}]", headers.join(", ")));
                self.pending_questions = Some(ask.questions.clone());
            }
            None => {}
        }
    }

    /// Set a captured error message.
    pub fn set_error(&mut self, msg: String) {
        self.error = Some(msg);
    }

    /// The captured error, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Pending questions from an `AskUserEvent`, if any.
    pub fn pending_questions(&self) -> Option<&[AskQuestion]> {
        self.pending_questions.as_deref()
    }

    /// Take and clear the pending questions.
    pub fn take_pending_questions(&mut self) -> Option<Vec<AskQuestion>> {
        self.pending_questions.take()
    }

    /// Render the current state: accumulated text + inline tool status.
    ///
    /// Returns the text to display in the chat message. If tools are
    /// currently running, appends the tool status line.
    pub fn render(&self) -> String {
        let mut out = self.text.clone();
        if let Some(ref line) = self.tool_line {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(line);
        }
        out
    }
}
