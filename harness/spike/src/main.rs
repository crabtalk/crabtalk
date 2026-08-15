//! Spike guest — the smallest thing that exercises the harness ABI.
//!
//! Host calls go through `rvtime-guest`; buffers are static rather than
//! heap-allocated, so the measurement reports the boundary's cost and not an
//! allocator's.

#![no_std]
#![no_main]

use core::panic::PanicInfo;
use rvtime_guest::{call0, call2};

const HOST_ARG_LEN: u64 = 1;
const HOST_ARG_READ: u64 = 2;

/// Traps instead of looping, so a guest bug reaches the host as
/// `Trap::Breakpoint` rather than hanging the thread that called in.
#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    rvtime_guest::abort()
}

/// A guest function returns at most two registers, which is exactly a pointer
/// and a length. `repr(C)` puts them in `a0` and `a1` under LP64.
#[repr(C)]
pub struct Buf {
    ptr: u64,
    len: u64,
}

const DESCRIPTION: &[u8] = br#"{"abi_version":0,"tools":[{"name":"echo","description":"Echo the argument blob back"}],"capabilities":[]}"#;

const PREFIX: &[u8] = br#"{"echo":""#;
const SUFFIX: &[u8] = br#""}"#;

static mut ARGS: [u8; 8192] = [0; 8192];
static mut OUT: [u8; 8192] = [0; 8192];

#[no_mangle]
pub static mut ANCHOR: u64 = 0;

/// The ELF entry point. Never called: it exists so `--gc-sections` keeps the
/// exports, which nothing else in the image references. A harness SDK has to
/// generate this — without it the linker discards every export and the host
/// rejects the guest as having no executable `.text`.
#[no_mangle]
pub extern "C" fn _start() {
    unsafe {
        core::ptr::write_volatile(
            &raw mut ANCHOR,
            describe as *const () as u64 ^ call as *const () as u64,
        );
    }
}

#[inline(never)]
#[no_mangle]
pub extern "C" fn describe() -> Buf {
    Buf {
        ptr: DESCRIPTION.as_ptr() as u64,
        len: DESCRIPTION.len() as u64,
    }
}

#[inline(never)]
#[no_mangle]
pub extern "C" fn call() -> Buf {
    unsafe {
        let args = core::ptr::addr_of_mut!(ARGS) as *mut u8;
        let out = core::ptr::addr_of_mut!(OUT) as *mut u8;

        let len = call0(HOST_ARG_LEN) as usize;
        let capacity = 8192 - PREFIX.len() - SUFFIX.len();
        let taken = if len < capacity { len } else { capacity };
        call2(HOST_ARG_READ, args as u64, taken as u64);

        let mut at = 0;
        core::ptr::copy_nonoverlapping(PREFIX.as_ptr(), out, PREFIX.len());
        at += PREFIX.len();
        core::ptr::copy_nonoverlapping(args, out.add(at), taken);
        at += taken;
        core::ptr::copy_nonoverlapping(SUFFIX.as_ptr(), out.add(at), SUFFIX.len());
        at += SUFFIX.len();

        Buf {
            ptr: out as u64,
            len: at as u64,
        }
    }
}
