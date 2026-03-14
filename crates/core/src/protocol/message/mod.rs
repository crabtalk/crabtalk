//! Wire protocol message types — enums, payload structs, and conversions.

pub mod client;
pub mod server;

pub use client::{ClientMessage, HubAction, HubRequest, MemoryOp, SendRequest, StreamRequest};
pub use server::{
    DownloadEvent, DownloadKind, EntityInfo, JournalInfo, MemoryResult, RelationInfo, SendResponse,
    ServerMessage, StreamEvent, TaskEvent,
};
