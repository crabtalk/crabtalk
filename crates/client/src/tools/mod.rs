//! Client tool declarations.
//!
//! A client's tools are exactly what it puts in `StreamMsg.tools`. The
//! daemon advertises that list and forwards the calls back via
//! `ToolCallForward`; the client executes and answers with `ReplyToTool`.
//! Declaring is therefore a promise to answer — the daemon holds no
//! fallback set, so a tool left undeclared is simply never offered.
//!
//! `ask_user` lives here because a UI answers it, which is the whole of the
//! remaining test: a client tool is one whose result requires a human to be
//! present. `bash`, `read`, and `edit` failed that test and became a harness
//! the runtime executes (RFC 0205).

pub mod ask_user;
