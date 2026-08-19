//! How a capability request is laid out.
//!
//! Every request is a sequence of length-prefixed byte fields — one rule with
//! no exceptions, so the host has a single decoder and a capability taking one
//! argument is framed the same way as one taking five. The four bytes a
//! single-field request spends on its own length buy that.
//!
//! A reply uses the same layout when it carries more than one blob, which is
//! why [`fields`] is here: the framing is the wire, not the direction.
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

/// Split a framed reply back into its fields. `None` if it is malformed,
/// which for a guest means the host is not the one it was built against.
pub(crate) fn fields(mut framed: &[u8]) -> Option<Vec<&[u8]>> {
    let mut fields = Vec::new();
    while !framed.is_empty() {
        let (header, rest) = framed.split_at_checked(4)?;
        let length = u32::from_le_bytes(header.try_into().ok()?) as usize;
        let (field, rest) = rest.split_at_checked(length)?;
        fields.push(field);
        framed = rest;
    }
    Some(fields)
}
