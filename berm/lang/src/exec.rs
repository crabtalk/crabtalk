//! Commands, run on the host under the same root `fs` is bounded by.
//!
//! A harness holding this can do anything the user can, and nothing here
//! narrows that — see RFC 0205. What bounds it is the grant.

use crate::{abi::HOST_EXEC_RUN, cap, wire};
use alloc::{string::String, vec::Vec};

/// Run `command` through a shell, in `cwd` relative to the granted root.
///
/// The result is a JSON object carrying `stdout`, `stderr`, and `exit_code`,
/// returned as bytes because its destination is a tool result: a harness hands
/// it straight back to the model rather than reading it. Parsing it only to
/// print it again is work nobody asked for.
pub fn run(command: &str, cwd: &str, env: &[(&str, &str)]) -> Result<Vec<u8>, String> {
    let mut request = wire::request(&[command.as_bytes(), cwd.as_bytes()]);
    for (key, value) in env {
        wire::field(&mut request, key.as_bytes());
        wire::field(&mut request, value.as_bytes());
    }
    cap::call(HOST_EXEC_RUN, &request)
}
