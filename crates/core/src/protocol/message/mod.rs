//! Wire protocol message types — enums, payload structs, and conversions.

pub mod client;
pub mod server;

pub use client::{
    ClientMessage, DownloadRequest, HubAction, HubRequest, Resource, SendRequest, StreamRequest,
};
pub use server::{
    DownloadEvent, DownloadKind, ResourceList, SendResponse, ServerMessage, SkillInfo, StreamEvent,
    TaskEvent,
};
