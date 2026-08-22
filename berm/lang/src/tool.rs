//! What every tool body does before and after its own work.
//!
//! Reading arguments and reporting a failure are the same in every harness,
//! and a copy per crate is a copy that drifts — one of them starts wording an
//! error differently, and a model sees two conventions from one runtime.
#![cfg(feature = "alloc")]

use crate::{Failed, Out};
use alloc::string::String;
use core::fmt::Write;

/// Deserialize a tool's arguments, reporting the error to the model rather
/// than trapping — a malformed call is the model's mistake to fix, not the
/// harness's to die of.
///
/// Arguments land in a struct rather than a dynamic value because the latter
/// reaches the guest's one unsupported construct, dynamic dispatch, and traps.
/// See `docs/src/rfcs/0205-berm.md`.
#[cfg(feature = "args")]
pub fn parse<T: for<'de> serde_guest::Deserialize<'de>>(
    args: &[u8],
    out: &mut Out,
) -> Result<T, Failed> {
    match serde_json_guest::from_slice(args) {
        Ok(parsed) => Ok(parsed),
        Err(error) => {
            let _ = write!(out, "invalid arguments: {error}");
            Err(Failed)
        }
    }
}

/// Turn a system harness's failure into this tool's failure, unchanged. The host
/// already said what went wrong, and in more detail than a rewording would
/// keep.
pub fn system<T>(result: Result<T, String>, out: &mut Out) -> Result<T, Failed> {
    result.map_err(|error| {
        out.write(error.as_bytes());
        Failed
    })
}

/// Fail with a message, for a tool that has decided its own arguments are
/// wrong before reaching a system harness.
pub fn failed(message: &str, out: &mut Out) -> Result<(), Failed> {
    out.write(message.as_bytes());
    Err(Failed)
}
