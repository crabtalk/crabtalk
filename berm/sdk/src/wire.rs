//! How a capability request is laid out.
//!
//! Every request is a sequence of length-prefixed byte fields — one rule with
//! no exceptions, so the host has a single decoder and a capability taking one
//! argument is framed the same way as one taking five. The four bytes a
//! single-field request spends on its own length buy that.
//!
//! Not JSON, because file content is arbitrary bytes and JSON cannot carry
//! those without base64. A capability that moves bytes should not pay an
//! encoding to do it.

use alloc::vec::Vec;

/// Append one length-prefixed field.
pub(crate) fn field(request: &mut Vec<u8>, bytes: &[u8]) {
    request.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    request.extend_from_slice(bytes);
}

/// Build a request from its fields.
pub(crate) fn request(fields: &[&[u8]]) -> Vec<u8> {
    let mut request = Vec::new();
    for bytes in fields {
        field(&mut request, bytes);
    }
    request
}
