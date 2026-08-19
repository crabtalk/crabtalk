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
//! #[berm_lang::harness]
//! mod tools {
//!     use berm_lang::{Failed, Out};
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
//! The system set here is the machine — files and commands, the things a
//! no_std RV64 guest cannot do for itself. A harness that talks to the Crabtalk
//! daemon adds [`berm-crabtalk`](https://crates.io/crates/berm-crabtalk), which
//! declares that namespace and carries the message types.

#![no_std]

// Off the guest's target this is an ordinary library in someone's test binary,
// where std is both available and the point: it is what lets a stand-in host
// hold the argument blob and collect what a harness logged.
#[cfg(not(target_arch = "riscv64"))]
extern crate std;

#[cfg(feature = "alloc")]
extern crate alloc;

// `harnesses!` emits `::berm_lang::` paths so an expansion works in a harness's
// own crate. That only resolves in here if the crate can name itself.
extern crate self as berm_lang;

/// The wire between a harness and its host: call numbers, framing, and the one
/// call path. Public because `harnesses!` emits `abi::hash` for every name it
/// generates, and hidden inside because the rest is reached only by generated
/// code — an expansion lands in the harness's crate, not this one.
pub mod abi;
// Installing a global allocator is something only the whole program may do,
// so like the panic handler it happens on the guest's target and nowhere else.
#[cfg(all(feature = "alloc", target_arch = "riscv64"))]
mod heap;
mod out;
/// Run a harness's tools natively. Present only off the guest's target, where
/// there is a test to run them from.
#[cfg(not(target_arch = "riscv64"))]
pub mod test;
/// Argument parsing and failure reporting, shared by every tool body.
#[cfg(feature = "alloc")]
mod tool;

// One boundary for the whole crate: `riscv.rs` makes real host calls, and
// `stub.rs` is a host a test can stand in for. The split is about behaviour
// rather than what will compile — `rvtime-guest` builds everywhere now — and it
// is what lets a harness author run their tools without a guest around them.
#[cfg_attr(target_arch = "riscv64", path = "sys/riscv.rs")]
#[cfg_attr(not(target_arch = "riscv64"), path = "sys/stub.rs")]
mod sys;

// The guest half of berm's system set. berm declares the host half of the same
// thing, in its own crate: both are published, and a proc macro cannot read a
// dependency's files, so a shared path would leave one package without it.
//
// Drift is caught rather than prevented. A renamed call hashes to a number
// nothing is registered for, and a changed field count fails the arity check
// the host expansion emits — both loud, on the first call, which
// `cargo run --example os -p berm` makes.
#[cfg(feature = "alloc")]
harnesses!(guest {
    namespace = "berm";

    /// Files, bounded by a granted root.
    mod fs {
        /// Read a file whole.
        fn read(path: &str) -> Vec<u8>;
        /// Write a file, replacing what was there.
        fn write(path: &str, content: &[u8]);
    }

    /// Commands, under the same root `fs` is bounded by.
    mod exec {
        /// Run a command through a shell, in `cwd` relative to the root.
        fn run(command: &str, cwd: &str, env: &[(&str, &str)]) -> Vec<u8>;
    }
});

pub use abi::{Buf, args_len, log};
pub use berm_codegen::{harness, harnesses};
pub use out::Out;
#[cfg(feature = "args")]
pub use tool::parse;
#[cfg(feature = "alloc")]
pub use tool::{failed, system};

// Re-exported so a harness declares this SDK and nothing else. The `#[harness]`
// macro writes `#[serde(crate = "::berm_lang::serde")]` onto argument structs,
// which only resolves if the author can reach serde through us — and an author
// who had to depend on it directly could pick a version that disagrees with
// the one the derive was generated against.
#[cfg(feature = "args")]
pub use serde_guest as serde;
#[cfg(feature = "args")]
pub use serde_json_guest as serde_json;

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
