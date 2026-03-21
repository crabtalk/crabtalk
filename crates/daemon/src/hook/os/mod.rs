//! OS hook — shell tool for agents.
//!
//! Registers the `bash` tool schema. Dispatch method lives on
//! [`DaemonHook`](crate::hook::DaemonHook).

use std::fmt::Write;

pub(crate) mod tool;

/// Build an `<environment>` XML block with OS info.
/// Appended to every agent's system prompt. Working directory is injected
/// per-session via `on_before_run` instead of here.
pub fn environment_block() -> String {
    let mut buf = String::from("\n\n<environment>\n");
    let _ = writeln!(buf, "os: {}", std::env::consts::OS);
    buf.push_str("</environment>");
    buf
}
