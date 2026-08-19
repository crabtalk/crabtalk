//! Requests, to the hosts the harness was granted.
//!
//! The allowlist is not visible here and cannot be: it lives in the
//! declaration and is enforced host-side, so a URL pointing somewhere the
//! harness was never granted comes back as an error rather than as something
//! this crate had to remember to check — the same arrangement `fs` has with
//! its root.
//!
//! Redirects are not followed for you. A 3xx arrives as a 3xx, and following
//! it means calling again with the new URL, which is checked like any other.

use crate::{abi::HOST_HTTP_FETCH, cap, wire};
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

/// What came back.
pub struct Response {
    /// HTTP status.
    pub status: u16,
    /// Response headers, one `name: value` line each, lowercased by the host.
    pub headers: String,
    /// The body, unparsed. It is as likely to be HTML or a compressed stream
    /// as it is to be text, so it stays bytes.
    pub body: Vec<u8>,
}

impl Response {
    /// The value of `name`, if the response carried it.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.lines().find_map(|line| {
            let (key, value) = line.split_once(": ")?;
            key.eq_ignore_ascii_case(name).then(|| value.trim())
        })
    }
}

/// Perform one request. `headers` are name/value pairs; `body` may be empty.
pub fn fetch(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Result<Response, String> {
    let mut request = wire::request(&[method.as_bytes(), url.as_bytes(), body]);
    for (name, value) in headers {
        wire::field(&mut request, name.as_bytes());
        wire::field(&mut request, value.as_bytes());
    }

    let reply = cap::call(HOST_HTTP_FETCH, &request)?;
    let Some(fields) = wire::fields(&reply) else {
        return Err(String::from("the host framed a reply this SDK cannot read"));
    };
    let [status, headers, body] = fields[..] else {
        return Err(String::from("the host's reply is not a response"));
    };

    Ok(Response {
        status: core::str::from_utf8(status)
            .ok()
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| String::from("the host sent a status that is not a number"))?,
        headers: String::from_utf8_lossy(headers).to_string(),
        body: body.to_vec(),
    })
}

/// Fetch a URL with GET and no headers.
pub fn get(url: &str) -> Result<Response, String> {
    fetch("GET", url, &[], &[])
}
