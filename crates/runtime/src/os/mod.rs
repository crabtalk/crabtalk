//! OS hook — shell tool for agents.

use std::fmt::Write;

pub mod tool;

/// Build an `<environment>` XML block with OS info.
pub fn environment_block() -> String {
    let mut buf = String::from("\n\n<environment>\n");
    let _ = writeln!(buf, "os: {}", std::env::consts::OS);
    buf.push_str("</environment>");
    buf
}
