//! Stream helpers — accumulator for chat-platform apps and an [`OutputChunk`]
//! adapter for richer UI consumers (TUI).

use crate::{
    conn::{ConnectionInfo, Transport, connect_from},
    tools::ask_user::{AskUser, Question},
};
use anyhow::Result;
use futures_core::Stream;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use wcore::protocol::{api::Client as _, message::*};

/// Accumulates streaming events into a renderable text buffer.
///
/// Used by chat-platform apps (telegram, wechat) where the UI is a single
/// text bubble updated in place.
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
    /// Pending structured questions from a forwarded `ask_user` call.
    pending_ask: Option<PendingAsk>,
}

/// A forwarded `ask_user` tool call awaiting a reply.
pub struct PendingAsk {
    pub questions: Vec<Question>,
    pub conversation_id: u64,
    pub call_id: String,
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
            pending_ask: None,
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
            stream_event::Event::ToolCallForward(fwd) if fwd.name == "ask_user" => {
                if let Ok(ask) = serde_json::from_str::<AskUser>(&fwd.arguments) {
                    let headers: Vec<&str> =
                        ask.questions.iter().map(|q| q.header.as_str()).collect();
                    self.tool_line = Some(format!("[question: {}]", headers.join(", ")));
                    self.pending_ask = Some(PendingAsk {
                        questions: ask.questions,
                        conversation_id: fwd.conversation_id,
                        call_id: fwd.call_id.clone(),
                    });
                }
            }
            stream_event::Event::AskUser(ask) => {
                let headers: Vec<&str> = ask.questions.iter().map(|q| q.header.as_str()).collect();
                self.tool_line = Some(format!("[question: {}]", headers.join(", ")));
            }
            stream_event::Event::ToolCallForward(_)
            | stream_event::Event::UserSteered(_)
            | stream_event::Event::ContextUsage(_)
            | stream_event::Event::TextStart(_)
            | stream_event::Event::TextEnd(_)
            | stream_event::Event::ThinkingStart(_)
            | stream_event::Event::ThinkingEnd(_) => {}
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

    /// Pending ask_user call, if any.
    pub fn pending_ask(&self) -> Option<&PendingAsk> {
        self.pending_ask.as_ref()
    }

    /// Take and clear the pending ask.
    pub fn take_pending_ask(&mut self) -> Option<PendingAsk> {
        self.pending_ask.take()
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
    /// Agent is asking the user structured questions via a forwarded
    /// `ask_user` tool call. Reply with `ReplyToTool` echoing
    /// `conversation_id` and `call_id`.
    AskUser {
        questions: Vec<Question>,
        conversation_id: u64,
        call_id: String,
    },
    /// Daemon forwarded a tool call for the client to dispatch locally.
    /// The client must respond by sending `ReplyToTool` on a fresh
    /// connection, echoing both `conversation_id` and `call_id`.
    ToolCallForward {
        conversation_id: u64,
        call_id: String,
        name: String,
        arguments: String,
    },
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
                Ok(stream_event::Event::ToolCallForward(fwd)) if fwd.name == "ask_user" => {
                    match serde_json::from_str::<AskUser>(&fwd.arguments) {
                        Ok(ask) => Some(Ok(OutputChunk::AskUser {
                            questions: ask.questions,
                            conversation_id: fwd.conversation_id,
                            call_id: fwd.call_id,
                        })),
                        Err(_) => Some(Ok(OutputChunk::ToolCallForward {
                            conversation_id: fwd.conversation_id,
                            call_id: fwd.call_id,
                            name: fwd.name,
                            arguments: fwd.arguments,
                        })),
                    }
                }
                Ok(stream_event::Event::ToolCallForward(fwd)) => {
                    Some(Ok(OutputChunk::ToolCallForward {
                        conversation_id: fwd.conversation_id,
                        call_id: fwd.call_id,
                        name: fwd.name,
                        arguments: fwd.arguments,
                    }))
                }
                Ok(stream_event::Event::TextStart(_)) => Some(Ok(OutputChunk::TextStart)),
                Ok(stream_event::Event::TextEnd(_)) => Some(Ok(OutputChunk::TextEnd)),
                Ok(stream_event::Event::ThinkingStart(_)) => Some(Ok(OutputChunk::ThinkingStart)),
                Ok(stream_event::Event::ThinkingEnd(_)) => Some(Ok(OutputChunk::ThinkingEnd)),
                Ok(stream_event::Event::Start(_))
                | Ok(stream_event::Event::AskUser(_))
                | Ok(stream_event::Event::UserSteered(_))
                | Ok(stream_event::Event::ContextUsage(_))
                | Ok(stream_event::Event::End(_)) => None,
                Err(e) => Some(Err(e)),
            };
            std::future::ready(Some(chunk))
        })
        .filter_map(std::future::ready)
}
