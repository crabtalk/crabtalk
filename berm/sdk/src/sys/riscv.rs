//! The guest's own target, where the host calls are real.

#[inline]
pub fn call0(number: u64) -> u64 {
    unsafe { rvtime_guest::call0(number) }
}

#[inline]
pub fn call2(number: u64, a0: u64, a1: u64) -> u64 {
    unsafe { rvtime_guest::call2(number, a0, a1) }
}
