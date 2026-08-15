//! The wire between a harness and its host.
//!
//! A capability is identified by its *name*; the number `ecall` carries in
//! `a7` is derived from it (RFC 0205). Adding a capability therefore cannot
//! collide with an allocation someone else made, and there is no registry of
//! integers to keep.
//!
//! The same hash is computed guest-side in `harness/sdk/src/abi.rs`. The two
//! must agree — and cannot drift quietly if they don't: a mismatched name
//! hashes to a number no closure is registered for, so the first call traps
//! as an unknown host call rather than reaching the wrong capability.

/// Write a UTF-8 message to the host log. `(ptr, len) -> 0`
pub const HOST_LOG: u64 = hash("crabtalk.log");
/// Byte length of this invocation's argument blob. `() -> len`
pub const HOST_ARG_LEN: u64 = hash("crabtalk.args.len");
/// Copy the argument blob into guest memory. `(ptr, cap) -> full length`
pub const HOST_ARG_READ: u64 = hash("crabtalk.args.read");
/// Fail this invocation with a message. `(ptr, len) -> 0`
pub const HOST_FAIL: u64 = hash("crabtalk.fail");

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

/// ELF section carrying the harness's manifest: ABI version, tools, and the
/// capabilities it wants. A section rather than an export, so reading what a
/// harness claims to be never means running it.
pub const ABI_SECTION: &str = ".crabtalk.abi";
/// Where this guest's heap starts. `() -> address`
pub const HOST_HEAP_START: u64 = hash("crabtalk.heap.start");
/// How many bytes of it there are. `() -> length`
pub const HOST_HEAP_SIZE: u64 = hash("crabtalk.heap.size");
/// Prefix on every tool's exported symbol. A tool is resolved by name like any
/// other symbol; the prefix keeps one called `init` from colliding with the
/// exports the ABI reserves.
pub const TOOL_PREFIX: &str = "crabtalk_tool_";
