//! Stream helpers — accumulator for chat-platform apps and an [`OutputChunk`]
//! adapter for richer UI consumers (TUI).

use crate::conn::{ConnectionInfo, Transport, connect_from};
use anyhow::Result;
use futures_core::Stream;
use futures_util::StreamExt;
use proto::api::Client as _;
use proto::*;
use tokio::sync::mpsc;

/// Accumulates streaming events into a renderable text buffer.
///
/// Used by chat-platform apps (telegram) where the UI is a single text
/// bubble updated in place.
pub struct StreamAccumulator {
    /// Accumulated response text.
    text: String,
    /// Current tool call status line (e.g., "[calling bash, read...]").
    tool_line: Option<String>,
    /// Agent name from StreamStart.
    pub agent: Option<String>,
    /// Captured error, if any.
    error: Option<String>,
    /// Whether the stream has ended.
    pub done: bool,
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
            agent: None,
            error: None,
            done: false,
        }
    }

    /// Push a stream event into the accumulator.
    pub fn push(&mut self, event: &stream_event::Event) {
        match event {
            stream_event::Event::Start(s) => {
                self.agent = Some(s.agent.clone());
            }
            stream_event::Event::Chunk(c) => {
                self.text.push_str(&c.content);
            }
            stream_event::Event::Thinking(_) => {}
            stream_event::Event::ToolStart(ts) => {
                let names: Vec<&str> = ts.calls.iter().map(|c| c.name.as_str()).collect();
                self.tool_line = Some(format!("[calling {}...]", names.join(", ")));
            }
            stream_event::Event::ToolResult(_) => {}
            stream_event::Event::ToolsComplete(_) => {
                self.tool_line = None;
            }
            stream_event::Event::End(end) => {
                if !end.error.is_empty() {
                    self.set_error(end.error.clone());
                }
                self.done = true;
            }
            stream_event::Event::UserSteered(_)
            | stream_event::Event::ContextUsage(_)
            | stream_event::Event::TextStart(_)
            | stream_event::Event::TextEnd(_)
            | stream_event::Event::ThinkingStart(_)
            | stream_event::Event::ThinkingEnd(_) => {}
        }
    }

    /// The accumulated response text, without the tool status line.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Set a captured error message.
    pub fn set_error(&mut self, msg: String) {
        self.error = Some(msg);
    }

    /// The captured error, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Render the current state: accumulated text + inline tool status.
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

/// A typed chunk from the streaming response.
pub enum OutputChunk {
    /// A text segment is starting.
    TextStart,
    /// Regular text content delta.
    Text(String),
    /// The current text segment has ended.
    TextEnd,
    /// A thinking segment is starting.
    ThinkingStart,
    /// Thinking/reasoning content delta (displayed dimmed).
    Thinking(String),
    /// The current thinking segment has ended.
    ThinkingEnd,
    /// Tool execution started with these tool calls (name, arguments JSON).
    ToolStart(Vec<(String, String)>),
    /// Tool result returned (call_id, output).
    ToolResult(String, String),
    /// Tool execution completed (true = success, false = failure).
    ToolDone(bool),
}

/// Open a fresh connection from `conn_info` and stream chunks for `req` over
/// an unbounded channel. The spawned task closes the connection when the
/// daemon ends the stream or the receiver is dropped.
pub fn spawn_stream(
    conn_info: ConnectionInfo,
    req: StreamMsg,
) -> mpsc::UnboundedReceiver<Result<OutputChunk>> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut transport = match connect_from(&conn_info).await {
            Ok(t) => t,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        let stream = stream_chunks(&mut transport, req);
        tokio::pin!(stream);
        while let Some(chunk) = stream.next().await {
            if tx.send(chunk).is_err() {
                break;
            }
        }
    });
    rx
}

/// Run a [`StreamMsg`] on `transport` and translate `stream_event::Event`
/// into UI-friendly [`OutputChunk`]s. Filters telemetry-only events
/// (`Start`, `End`, `ContextUsage`, `UserSteered`).
pub fn stream_chunks<'a>(
    transport: &'a mut Transport,
    req: StreamMsg,
) -> impl Stream<Item = Result<OutputChunk>> + Send + 'a {
    transport
        .stream(req)
        .scan((), |_state, result| {
            let chunk = match result {
                Ok(stream_event::Event::Chunk(c)) => Some(Ok(OutputChunk::Text(c.content))),
                Ok(stream_event::Event::Thinking(t)) => Some(Ok(OutputChunk::Thinking(t.content))),
                Ok(stream_event::Event::ToolStart(ts)) => {
                    let calls = ts
                        .calls
                        .into_iter()
                        .map(|c| (c.name, c.arguments))
                        .collect();
                    Some(Ok(OutputChunk::ToolStart(calls)))
                }
                Ok(stream_event::Event::ToolResult(tr)) => {
                    Some(Ok(OutputChunk::ToolResult(tr.call_id, tr.output)))
                }
                Ok(stream_event::Event::ToolsComplete(_)) => Some(Ok(OutputChunk::ToolDone(true))),
                Ok(stream_event::Event::TextStart(_)) => Some(Ok(OutputChunk::TextStart)),
                Ok(stream_event::Event::TextEnd(_)) => Some(Ok(OutputChunk::TextEnd)),
                Ok(stream_event::Event::ThinkingStart(_)) => Some(Ok(OutputChunk::ThinkingStart)),
                Ok(stream_event::Event::ThinkingEnd(_)) => Some(Ok(OutputChunk::ThinkingEnd)),
                Ok(stream_event::Event::Start(_))
                | Ok(stream_event::Event::UserSteered(_))
                | Ok(stream_event::Event::ContextUsage(_))
                | Ok(stream_event::Event::End(_)) => None,
                Err(e) => Some(Err(e)),
            };
            std::future::ready(Some(chunk))
        })
        .filter_map(std::future::ready)
}
