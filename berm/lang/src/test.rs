//! Run a harness's tools natively, without a toolchain or a daemon.
//!
//! A harness is an ELF that a host compiles and enters, which is a slow way to
//! find out that a handler mishandles an empty string. Off the guest's target
//! the exports are ordinary functions and the buffers are ordinary memory, so a
//! test can call one the way the host would — the same argument transfer, the
//! same buffer limits, the same failure channel — and check what came back.
//!
//! ```ignore
//! #[test]
//! fn echo_wraps_the_payload() {
//!     let out = berm_lang::test::call(berm_tool_echo, b"hi").unwrap();
//!     assert_eq!(out, br#"{"echo":hi}"#);
//! }
//! ```
#![cfg(not(target_arch = "riscv64"))]

use crate::{Buf, sys};
use std::{string::String, vec::Vec};

/// Invoke a tool export with `args`, as the host does.
///
/// `Err` is what the harness reported through its failure channel — the same
/// distinction the host draws between a tool that failed and one that returned
/// the word "error".
pub fn call(entry: extern "C" fn() -> Buf, args: &[u8]) -> Result<Vec<u8>, String> {
    sys::with(|host| {
        host.args = args.to_vec();
        host.logged.clear();
        host.failure = None;
    });

    let Buf { ptr, len } = entry();

    if let Some(failure) = sys::with(|host| host.failure.take()) {
        return Err(failure);
    }
    // Safety: the harness returned this pair to be read, and off-target its
    // buffers are ordinary memory in this process.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
    Ok(bytes.to_vec())
}

/// What the harness logged during the last [`call`].
pub fn logged() -> Vec<String> {
    sys::with(|host| host.logged.clone())
}
