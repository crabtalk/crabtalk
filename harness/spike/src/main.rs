//! Spike guest — the smallest real harness, used to measure the boundary.
//!
//! Everything below the `#[harness]` line is what an author actually writes.
//! The exports, the description, the dispatch, and the panic handler come from
//! the SDK.

#![no_std]
#![no_main]

#[crabtalk_harness_sdk::harness(capabilities = [])]
mod tools {
    use crabtalk_harness_sdk::{Failed, Out};

    /// Echo the argument blob back inside a JSON envelope.
    #[params(r#"{"type":"object","properties":{"query":{"type":"string"}}}"#)]
    pub fn echo(args: &[u8], out: &mut Out) -> Result<(), Failed> {
        out.write(br#"{"echo":"#);
        out.write(args);
        out.write(b"}");
        Ok(())
    }

    /// Always fails, to exercise the error path.
    pub fn boom(_args: &[u8], out: &mut Out) -> Result<(), Failed> {
        out.write(b"boom, as requested");
        Err(Failed)
    }
}
