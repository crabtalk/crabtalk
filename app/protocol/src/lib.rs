//! Walrus wire protocol types shared between gateway and client.

use compact_str::CompactString;
use serde::{Deserialize, Serialize};

/// Current protocol version.
pub const PROTOCOL_VERSION: &str = "0.1";

/// Messages sent by the client to the gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Authenticate with the gateway.
    Authenticate {
        /// Authentication token.
        token: String,
    },
    /// Send a message to an agent and receive a complete response.
    Send {
        /// Target agent identifier.
        agent: CompactString,
        /// Message content.
        content: String,
    },
    /// Send a message to an agent and receive a streamed response.
    Stream {
        /// Target agent identifier.
        agent: CompactString,
        /// Message content.
        content: String,
    },
    /// Clear the session history for an agent.
    ClearSession {
        /// Target agent identifier.
        agent: CompactString,
    },
    /// Ping the server (keepalive).
    Ping,
}

/// Messages sent by the gateway to the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Authentication succeeded.
    Authenticated {
        /// Unique session identifier.
        session_id: CompactString,
    },
    /// Complete response from an agent.
    Response {
        /// Source agent identifier.
        agent: CompactString,
        /// Response content.
        content: String,
    },
    /// Start of a streamed response.
    StreamStart {
        /// Source agent identifier.
        agent: CompactString,
    },
    /// A chunk of streamed content.
    StreamChunk {
        /// Chunk content.
        content: String,
    },
    /// End of a streamed response.
    StreamEnd {
        /// Source agent identifier.
        agent: CompactString,
    },
    /// Session cleared for an agent.
    SessionCleared {
        /// Agent whose session was cleared.
        agent: CompactString,
    },
    /// Error response.
    Error {
        /// Error code.
        code: u16,
        /// Error message.
        message: String,
    },
    /// Pong response to client ping.
    Pong,
}
