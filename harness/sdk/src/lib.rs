//! Build a Crabtalk harness.
//!
//! A harness is code the daemon schedules: one RV64IMAC ELF, confined to its
//! own address space, reaching the world only through host calls it was
//! granted. This crate is what an author writes against — it owns the ABI so
//! they never see a call number, a register, or a pointer pair.
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//!
//! #[crabtalk_harness_sdk::harness(capabilities = ["log"])]
//! mod tools {
//!     use crabtalk_harness_sdk::{Failed, Out};
//!
//!     /// Echo the argument blob back.
//!     pub fn echo(args: &[u8], out: &mut Out) -> Result<(), Failed> {
//!         out.write(args);
//!         Ok(())
//!     }
//! }
//! ```
//!
//! Every `pub fn` in the module is a tool; its doc comment is what the model
//! reads when deciding whether to call it.
//!
//! Build with the `riscv64imac-unknown-none-elf` target and
//! `-Clink-arg=--emit-relocs`; the template's `.cargo/config.toml` carries
//! both, and neither is optional.
//!
//! This is not [`crabtalk-client`](https://crates.io/crates/crabtalk-client),
//! which connects to the daemon over a socket. Both speak the same protocol;
//! only this one runs inside the sandbox.

#![no_std]

// Off the guest's target this is an ordinary library in someone's test binary,
// where std is both available and the point: it is what lets a stand-in host
// hold the argument blob and collect what a harness logged.
#[cfg(not(target_arch = "riscv64"))]
extern crate std;

#[cfg(feature = "alloc")]
extern crate alloc;

mod abi;
/// One call path shared by every capability. Needs a heap: a result whose size
/// the guest learns at runtime has nowhere else to go.
#[cfg(feature = "alloc")]
mod cap;
/// Commands. Granted as `exec`.
#[cfg(feature = "alloc")]
pub mod exec;
/// Files, bounded by the granted root. Granted as `fs`.
#[cfg(feature = "alloc")]
pub mod fs;
/// The runtime, over `ClientMessage`. Granted as `protocol:<group>`.
#[cfg(feature = "protocol")]
pub mod protocol;
// Installing a global allocator is something only the whole program may do,
// so like the panic handler it happens on the guest's target and nowhere else.
#[cfg(all(feature = "alloc", target_arch = "riscv64"))]
mod heap;
mod out;
/// Run a harness's tools natively. Present only off the guest's target, where
/// there is a test to run them from.
#[cfg(not(target_arch = "riscv64"))]
pub mod test;
#[cfg(feature = "alloc")]
mod wire;

// One boundary for the whole crate: `riscv.rs` makes real host calls, and
// `stub.rs` is a host a test can stand in for. The split is about behaviour
// rather than what will compile — `rvtime-guest` builds everywhere now — and it
// is what lets a harness author run their tools without a guest around them.
#[cfg_attr(target_arch = "riscv64", path = "sys/riscv.rs")]
#[cfg_attr(not(target_arch = "riscv64"), path = "sys/stub.rs")]
mod sys;

pub use abi::{Buf, args_len, log};
pub use crabtalk_harness_codegen::harness;
pub use out::Out;

/// The ABI this SDK generates against. A host that does not recognise it
/// refuses the harness rather than guessing.
pub const ABI_VERSION: u32 = 0;

#[doc(hidden)]
pub const ABI_VERSION_TEXT: &str = "0";

/// Returned by a handler that failed. Whatever it wrote to its [`Out`] becomes
/// the failure message, so an error can be specific without an allocator.
pub struct Failed;

/// Traps instead of looping. An author's panic reaches the host as a
/// breakpoint it can report, rather than hanging the thread that called in.
///
/// Only on the guest's own target — off it, the crate is an ordinary library
/// linked into someone's tests, where std already supplies one.
#[cfg(target_arch = "riscv64")]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // Say where before dying. A harness author otherwise gets `guest executed
    // ebreak` and nothing else, which is the difference between a minute and
    // an afternoon.
    if let Some(location) = info.location() {
        abi::log(location.file());
    }
    rvtime_guest::abort()
}

#[doc(hidden)]
pub fn read_args(buffer: &mut [u8]) -> usize {
    abi::read_args(buffer)
}

#[doc(hidden)]
pub fn fail(message: &[u8]) -> Buf {
    abi::fail(message)
}
