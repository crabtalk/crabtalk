//! The runtime, over one call.
//!
//! Everything a harness does *to* the runtime is already a `ClientMessage`,
//! so there is no second vocabulary here — a harness sends the same message a
//! client would and gets the same reply (RFC 0205). What it is *allowed* to
//! send is the declaration's business, checked host-side on decode; an
//! ungranted message type comes back as an error rather than being silently
//! ignored.

use crate::{abi::HOST_PROTOCOL_CALL, cap};
use alloc::{string::String, vec::Vec};
use crabtalk_harness_proto::{ClientMessage, ServerMessage};
use prost_guest::Message;

/// Send one message and wait for its reply.
///
/// Only request-response message types can be granted today, so one reply is
/// the whole answer.
pub fn call(message: ClientMessage) -> Result<ServerMessage, String> {
    let mut request = Vec::new();
    message
        .encode(&mut request)
        .map_err(|_| String::from("could not encode the request"))?;

    let reply = cap::call(HOST_PROTOCOL_CALL, &request)?;
    ServerMessage::decode(&reply[..]).map_err(|_| String::from("could not decode the reply"))
}
