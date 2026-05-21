//! Schema and types for the `ask_user` client tool.
//!
//! The daemon advertises this tool and forwards calls via
//! `ToolCallForward`; the client renders the question and replies
//! via `ReplyToTool`. OS tools (bash, read, edit) live in
//! `crabtalk-hooks::os`.

pub mod ask_user;
