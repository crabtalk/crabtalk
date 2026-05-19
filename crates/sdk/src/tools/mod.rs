//! Built-in tool implementations reusable across crabtalk hosts.
//!
//! OS tools (bash, read, edit) and ask_user are client-side tools:
//! the daemon forwards calls via `ToolCallForward`, the client
//! dispatches locally and replies via `ReplyToTool`.

pub mod ask_user;
pub mod os;
