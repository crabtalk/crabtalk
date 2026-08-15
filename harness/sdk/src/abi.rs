//! The wire between a harness and its host.
//!
//! Host calls travel in `a7`, which is a number — but the number is *derived*
//! from a name rather than assigned (RFC 0205). A capability is identified by
//! what it is called, so adding one cannot collide with someone else's
//! allocation and no registry of integers has to be maintained.
//!
//! The same hash is computed host-side in `crates/harness/src/abi.rs`. The two
//! must agree; if they ever drift, every call traps immediately as an unknown
//! host call rather than reaching the wrong capability.

use rvtime_guest::{call0, call2};

/// Write a UTF-8 message to the host log.
const HOST_LOG: u64 = hash("crabtalk.log");
/// Byte length of this invocation's argument blob.
const HOST_ARG_LEN: u64 = hash("crabtalk.args.len");
/// Copy the argument blob into guest memory.
const HOST_ARG_READ: u64 = hash("crabtalk.args.read");
/// Fail this invocation with a message.
const HOST_FAIL: u64 = hash("crabtalk.fail");

/// FNV-1a over the capability's name, evaluated at compile time.
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
    unsafe { call2(HOST_LOG, message.as_ptr() as u64, message.len() as u64) };
}

/// How many bytes this invocation was given.
pub fn args_len() -> usize {
    unsafe { call0(HOST_ARG_LEN) as usize }
}

/// Pull the argument blob into `buffer`, returning the blob's *full* length —
/// not what fit. A caller that gets back more than it offered was truncated
/// and must say so rather than acting on half a request.
pub fn read_args(buffer: &mut [u8]) -> usize {
    unsafe {
        call2(
            HOST_ARG_READ,
            buffer.as_mut_ptr() as u64,
            buffer.len() as u64,
        ) as usize
    }
}

/// Report failure. The host marks the invocation an error rather than a
/// result, which is the difference between a tool that failed and a tool that
/// returned the word "error".
pub fn fail(message: &[u8]) -> Buf {
    unsafe { call2(HOST_FAIL, message.as_ptr() as u64, message.len() as u64) };
    Buf { ptr: 0, len: 0 }
}
