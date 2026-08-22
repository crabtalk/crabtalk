//! What a host call does when there is no host.
//!
//! Off RISC-V this crate is an ordinary library, so a harness author can build
//! and unit test their handlers natively rather than cross-compiling to see
//! anything at all. The calls a test can reasonably answer — the argument blob,
//! the log, the failure channel — are served from [`crate::test`]'s state.
//! Anything else panics naming the system harness, because a test that reached a
//! real system harness should say so rather than read a plausible zero.

use crate::abi;
use std::{cell::RefCell, string::String, vec::Vec};

std::thread_local! {
    static HOST: RefCell<Host> = const { RefCell::new(Host::new()) };
}

/// The bits of a host a test can stand in for.
pub(crate) struct Host {
    pub args: Vec<u8>,
    pub logged: Vec<String>,
    pub failure: Option<String>,
}

impl Host {
    const fn new() -> Self {
        Self {
            args: Vec::new(),
            logged: Vec::new(),
            failure: None,
        }
    }
}

/// Run `f` against the thread's stand-in host.
pub(crate) fn with<T>(f: impl FnOnce(&mut Host) -> T) -> T {
    HOST.with(|host| f(&mut host.borrow_mut()))
}

#[inline]
pub fn call0(number: u64) -> u64 {
    match number {
        abi::HOST_ARG_LEN => with(|host| host.args.len() as u64),
        _ => no_host(number),
    }
}

#[inline]
pub fn call2(number: u64, a0: u64, a1: u64) -> u64 {
    match number {
        abi::HOST_ARG_READ => with(|host| {
            let taken = host.args.len().min(a1 as usize);
            // Safety: a0/a1 describe a buffer the caller owns, exactly as the
            // real host requires. Off-target that buffer is ordinary memory.
            unsafe { core::ptr::copy_nonoverlapping(host.args.as_ptr(), a0 as *mut u8, taken) };
            host.args.len() as u64
        }),
        abi::HOST_LOG => {
            with(|host| host.logged.push(read(a0, a1)));
            0
        }
        abi::HOST_FAIL => {
            with(|host| host.failure = Some(read(a0, a1)));
            0
        }
        _ => no_host(number),
    }
}

/// Read a `(ptr, len)` pair the caller passed. Off-target these are real
/// addresses in this process.
fn read(ptr: u64, len: u64) -> String {
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
    String::from_utf8_lossy(bytes).into_owned()
}

#[cold]
fn no_host(number: u64) -> ! {
    panic!("host call {number:#x} has no stand-in; it needs a real host");
}
