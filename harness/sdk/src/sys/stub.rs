//! What a host call does when there is no host.
//!
//! Off RISC-V this crate is an ordinary library, so a harness author can build
//! and unit test their own handlers natively rather than cross-compiling to see
//! anything at all. A call that actually reaches the boundary panics naming the
//! capability, so a test that wandered across it says which one instead of
//! reading a plausible zero.

#[inline]
pub fn call0(number: u64) -> u64 {
    no_host(number)
}

#[inline]
pub fn call2(number: u64, _a0: u64, _a1: u64) -> u64 {
    no_host(number)
}

/// No allocator is installed off-target: std already has one.
#[cfg(feature = "alloc")]
#[inline]
pub unsafe fn heap_init(_start: usize, _size: usize) {}

#[cold]
fn no_host(number: u64) -> ! {
    panic!("host call {number:#x} outside a guest");
}
