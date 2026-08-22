//! How a system harness call works, once, for all of them.
//!
//! A call hands the host a request and gets back a length. The bytes stay
//! host-side until the guest asks for them, which is the same pull the
//! argument blob uses (`args.len` then `args.read`) and is here for the same
//! reason: the host never enters a guest to give it something, and a result
//! whose size is unknown in advance cannot be measured by running the work
//! twice.
//!
//! Failure travels on the same wire. The high bit of the returned length says
//! the staged bytes are a message rather than a result, so an error costs no
//! extra call and cannot be mistaken for an empty success.

use crate::{
    abi::{ERROR, read_result},
    sys,
};
use alloc::{string::String, vec, vec::Vec};

/// Make one system harness call. `Err` carries whatever the host said went wrong.
pub fn call(number: u64, request: &[u8]) -> Result<Vec<u8>, String> {
    let staged = sys::call2(number, request.as_ptr() as u64, request.len() as u64);
    let failed = staged & ERROR != 0;

    let mut result = vec![0u8; (staged & !ERROR) as usize];
    let full = read_result(&mut result);
    if full != result.len() {
        return Err(String::from("host staged a result of a different length"));
    }

    if failed {
        return Err(String::from_utf8_lossy(&result).into_owned());
    }
    Ok(result)
}
