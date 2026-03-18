//! OS hook — shell tool for agents.
//!
//! Registers the `bash` tool schema. Dispatch method lives on
//! [`DaemonHook`](crate::hook::DaemonHook). Access control is handled by
//! the permission layer in `dispatch_tool`.

use std::fmt::Write;

pub use config::{PermissionConfig, ToolPermission};

pub mod config;
pub(crate) mod tool;

/// Build an `<environment>` XML block with OS, working directory, and sandbox
/// state. Appended to every agent's system prompt.
pub fn environment_block(sandboxed: bool) -> String {
    let mut buf = String::from("\n\n<environment>\n");
    let _ = writeln!(buf, "os: {}", std::env::consts::OS);
    let _ = writeln!(
        buf,
        "working_directory: {}",
        wcore::paths::HOME_DIR.display()
    );
    let _ = writeln!(
        buf,
        "sandbox: {}",
        if sandboxed { "active" } else { "inactive" }
    );
    buf.push_str("</environment>");
    buf
}
