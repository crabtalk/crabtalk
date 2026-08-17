//! Crabtalk client — sugar layer over `crates/transport`. Apps (TUI,
//! telegram, …) connect to the daemon through this crate.

use std::{collections::HashSet, sync::Arc};
use tokio::sync::RwLock;

pub mod command;
pub mod conn;
pub mod message;
pub mod stream;

pub use command::{COMMAND_HINT, COMMANDS, Command, collect_candidates, parse_command};
#[cfg(unix)]
pub use conn::connect_uds;
pub use conn::{ConnectionInfo, Transport, connect_from, connect_tcp};
pub use message::{Attachment, AttachmentKind, Message, attachment_summary};
pub use stream::{OutputChunk, StreamAccumulator, spawn_stream, stream_chunks};

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
