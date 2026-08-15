//! Decoding a capability request.
//!
//! The guest builds these in `berm/sdk/src/wire.rs`: a flat sequence of
//! `u32`-prefixed fields, one rule for every capability. This is the only
//! decoder, so a malformed request is rejected in one place rather than in
//! each capability's own hand-rolled parse.

use anyhow::{Result, bail};

/// Split a request into its fields.
pub fn fields(mut request: &[u8]) -> Result<Vec<&[u8]>> {
    let mut fields = Vec::new();
    while !request.is_empty() {
        let Some((header, rest)) = request.split_at_checked(4) else {
            bail!("truncated field header");
        };
        let length = u32::from_le_bytes(header.try_into()?) as usize;
        let Some((field, rest)) = rest.split_at_checked(length) else {
            bail!(
                "field claims {length} bytes but the request has {}",
                rest.len()
            );
        };
        fields.push(field);
        request = rest;
    }
    Ok(fields)
}

/// Read a field as UTF-8.
pub fn text<'a>(fields: &[&'a [u8]], at: usize, what: &str) -> Result<&'a str> {
    let Some(field) = fields.get(at) else {
        bail!("request has no {what}");
    };
    Ok(std::str::from_utf8(field)?)
}
