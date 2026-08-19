//! The wire between a harness and its host.
//!
//! Host calls travel in `a7`, which is a number — but the number is *derived*
//! from a name rather than assigned (RFC 0205). A system harness is identified by
//! what it is called, so adding one cannot collide with someone else's
//! allocation and no registry of integers has to be maintained.
//!
//! The same hash is computed host-side in `berm/berm/src/abi.rs`. The two
//! must agree; if they ever drift, every call traps immediately as an unknown
//! host call rather than reaching the wrong system harness.
//!
//! Whether a call reaches a host at all is `sys`'s business, not this file's.

use crate::sys;

/// Write a UTF-8 message to the host log.
pub const HOST_LOG: u64 = hash("berm.log");
/// Byte length of this invocation's argument blob.
pub const HOST_ARG_LEN: u64 = hash("berm.args.len");
/// Copy the argument blob into guest memory.
pub const HOST_ARG_READ: u64 = hash("berm.args.read");
/// Fail this invocation with a message.
pub const HOST_FAIL: u64 = hash("berm.fail");
/// Copy the last system harness call's staged result into guest memory.
pub const HOST_RESULT_READ: u64 = hash("berm.result.read");

/// Send one `ClientMessage`; the reply is a `ServerMessage`. Gated with the
/// module that calls it — the SDK builds without the protocol, for a harness
/// that only reaches the machine.
#[cfg(feature = "protocol")]
pub const HOST_PROTOCOL_CALL: u64 = hash("crabtalk.protocol.call");

/// Set on the length a system harness returns when the staged bytes are an error
/// message rather than a result. A length never reaches this bit on its own,
/// so one return value carries both without a second call to ask which.
pub(crate) const ERROR: u64 = 1 << 63;

/// FNV-1a over the system harness's name, evaluated at compile time.
pub const fn hash(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut result: u64 = 0xcbf2_9ce4_8422_2325;
    let mut at = 0;
    while at < bytes.len() {
        result ^= bytes[at] as u64;
        result = result.wrapping_mul(0x0000_0100_0000_01b3);
        at += 1;
    }
    result
}

/// A pointer and length, which is exactly what two result registers hold.
/// `repr(C)` puts them in `a0` and `a1` under LP64.
#[repr(C)]
pub struct Buf {
    pub ptr: u64,
    pub len: u64,
}

impl Buf {
    /// Point at bytes the host may read after the guest returns. They must
    /// outlive the call — a static buffer or a leaked allocation, never a
    /// local.
    pub fn new(bytes: &[u8]) -> Self {
        Self {
            ptr: bytes.as_ptr() as u64,
            len: bytes.len() as u64,
        }
    }
}

/// Write a line to the host's log.
pub fn log(message: &str) {
    sys::call2(HOST_LOG, message.as_ptr() as u64, message.len() as u64);
}

/// How many bytes this invocation was given.
pub fn args_len() -> usize {
    sys::call0(HOST_ARG_LEN) as usize
}

/// Pull the argument blob into `buffer`, returning the blob's *full* length —
/// not what fit. A caller that gets back more than it offered was truncated
/// and must say so rather than acting on half a request.
pub fn read_args(buffer: &mut [u8]) -> usize {
    sys::call2(
        HOST_ARG_READ,
        buffer.as_mut_ptr() as u64,
        buffer.len() as u64,
    ) as usize
}

/// Report failure. The host marks the invocation an error rather than a
/// result, which is the difference between a tool that failed and a tool that
/// returned the word "error".
pub fn fail(message: &[u8]) -> Buf {
    sys::call2(HOST_FAIL, message.as_ptr() as u64, message.len() as u64);
    Buf { ptr: 0, len: 0 }
}

/// Pull the last system harness call's staged result, returning its *full* length
/// exactly as [`read_args`] does — the one pattern both use.
pub(crate) fn read_result(buffer: &mut [u8]) -> usize {
    sys::call2(
        HOST_RESULT_READ,
        buffer.as_mut_ptr() as u64,
        buffer.len() as u64,
    ) as usize
}
