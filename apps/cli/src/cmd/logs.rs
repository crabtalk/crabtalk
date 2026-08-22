//! What was said, or what is being said.
//!
//! `docker logs` prints a container's past output and `-f` keeps the pipe
//! open. The daemon stores messages and broadcasts events, so those are two
//! calls rather than one: a handle reads the conversation, `--follow` opens
//! the event stream.

use crate::conn;
use anyhow::{Result, bail};
use futures_util::StreamExt;
use proto::{AgentEventKind, api::Client as _};

pub async fn run(handle: Option<String>, follow: bool) -> Result<()> {
    let mut transport = conn::connect().await?;

    if follow {
        let mut events = std::pin::pin!(transport.subscribe_events());
        while let Some(event) = events.next().await {
            let event = event?;
            if handle.as_ref().is_some_and(|h| h != &event.sender) {
                continue;
            }
            if let Some(line) = render(&event) {
                println!("{line}");
            }
        }
        return Ok(());
    }

    let Some(handle) = handle else {
        bail!("give a session handle, or --follow for live events");
    };
    let history = transport.get_conversation_history(handle).await?;
    println!("{} — {}", history.agent_name, history.title);
    for message in history.messages {
        println!("{}: {}", message.role, message.content);
    }
    Ok(())
}

/// One line per event worth a line. The deltas that build a single reply are
/// dropped: this is a log, not a renderer.
fn render(event: &proto::AgentEventMsg) -> Option<String> {
    let kind = AgentEventKind::try_from(event.kind).ok()?;
    let body = match kind {
        AgentEventKind::ToolStart => format!("tool {}", event.content),
        AgentEventKind::ToolResult => format!("tool done in {}", event.content),
        AgentEventKind::Done => format!("done — {}", event.content),
        _ => return None,
    };
    Some(format!("{} {} {body}", event.timestamp, event.agent))
}
