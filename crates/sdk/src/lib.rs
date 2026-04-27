//! Crabtalk SDK — sugar layer over `crates/transport`. Apps (TUI, telegram,
//! wechat, …) connect to the daemon through this crate.

use std::{collections::HashSet, path::Path, sync::Arc};
use tokio::sync::RwLock;

pub mod client;
pub mod command;
pub mod message;
pub mod stream;

#[cfg(unix)]
pub use client::connect_uds;
pub use client::{
    ConnectionInfo, MemConnection, OutputChunk, Transport, connect_from, connect_mem, connect_tcp,
    spawn_stream, stream_chunks,
};
pub use command::{COMMAND_HINT, COMMANDS, Command, collect_candidates, parse_command};
pub use message::{Attachment, AttachmentKind, Message, attachment_summary};
pub use stream::StreamAccumulator;

/// Shared set of sender IDs belonging to sibling Crabtalk bots.
///
/// Built incrementally as each bot connects. Channel loops check this set
/// before dispatching messages — senders in this set are silently dropped
/// to prevent agent-to-agent loops.
pub type KnownBots = Arc<RwLock<HashSet<String>>>;

/// Result of a streaming request to the daemon.
pub enum StreamResult {
    Ok,
    ConversationError,
    Failed,
}

/// Read the agents directory and return the first agent name found,
/// falling back to [`wcore::paths::DEFAULT_AGENT`].
pub fn resolve_default_agent(agents_dir: &Path) -> String {
    let Ok(entries) = std::fs::read_dir(agents_dir) else {
        return wcore::paths::DEFAULT_AGENT.to_owned();
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "md")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            return stem.to_owned();
        }
    }
    wcore::paths::DEFAULT_AGENT.to_owned()
}
