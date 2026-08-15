//! Client tool declarations.
//!
//! A client's tools are exactly what it puts in `StreamMsg.tools`. The
//! daemon advertises that list and forwards the calls back via
//! `ToolCallForward`; the client executes and answers with `ReplyToTool`.
//! Declaring is therefore a promise to answer — the daemon holds no
//! fallback set, so a tool left undeclared is simply never offered.
//!
//! `ask_user` lives here because a UI answers it. OS tools (bash, read,
//! edit) live in `crabtalk-hooks::os`.

pub mod ask_user;
