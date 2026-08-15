//! The harness ABI — host call numbers and required exports.
//!
//! Numbers are permanent: once a harness ships an ELF calling one, it means
//! that forever. See RFC 0205.

/// Write a UTF-8 message to the host log. `(ptr, len) -> 0`
pub const HOST_LOG: u64 = 0;
/// Byte length of this invocation's argument blob. `() -> len`
pub const HOST_ARG_LEN: u64 = 1;
/// Copy the argument blob into guest memory. `(ptr, cap) -> written`
pub const HOST_ARG_READ: u64 = 2;

/// Reports the harness ABI version, its tools, and the capabilities it wants.
pub const EXPORT_DESCRIBE: &str = "describe";
/// Runs one invocation. Arguments are pulled through [`HOST_ARG_READ`].
pub const EXPORT_CALL: &str = "call";
