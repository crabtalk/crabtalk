//! Where a handler writes its result.
//!
//! A fixed buffer rather than an allocation, so a harness that never needs a
//! heap never pays for one and the result stays valid after the handler
//! returns — the host reads it once the guest is back out.

use crate::abi::Buf;

/// A bounded sink over a caller-owned buffer.
///
/// Writes past the end are dropped and remembered: [`Out::overflowed`] is how
/// the generated `call` decides between returning a result and failing, so a
/// truncated payload never reaches the model looking complete.
pub struct Out<'a> {
    buffer: &'a mut [u8],
    at: usize,
    overflowed: bool,
}

impl<'a> Out<'a> {
    pub fn new(buffer: &'a mut [u8]) -> Self {
        Self {
            buffer,
            at: 0,
            overflowed: false,
        }
    }

    /// Append bytes, dropping any that do not fit.
    pub fn write(&mut self, bytes: &[u8]) {
        let room = self.buffer.len() - self.at;
        if bytes.len() > room {
            self.overflowed = true;
        }
        let n = room.min(bytes.len());
        self.buffer[self.at..self.at + n].copy_from_slice(&bytes[..n]);
        self.at += n;
    }

    /// Whether anything was dropped.
    pub fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// What has been written so far.
    pub fn written(&self) -> &[u8] {
        &self.buffer[..self.at]
    }

    /// Hand what was written back to the host.
    pub fn finish(&self) -> Buf {
        Buf::new(self.written())
    }
}

/// So handlers can `write!(out, "{{\"count\":{n}}}")` without an allocator.
impl core::fmt::Write for Out<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.write(s.as_bytes());
        Ok(())
    }
}
