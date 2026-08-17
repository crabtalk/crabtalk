//! Generate the protocol types a second time, for the harness.
//!
//! Same `.proto` as `crates/core`, different world: the host's copy is `std`
//! and carries serde derives, and a harness has neither. RFC 0205 records this
//! as duplication a build script keeps honest — one schema, two emissions,
//! and no way for them to drift without the file changing.

use std::io::Result;

fn main() -> Result<()> {
    let proto = "../../core/proto/crabtalk.proto";
    println!("cargo:rerun-if-changed={proto}");
    prost_build::Config::new()
        // `map` fields default to `std::collections::HashMap`, which a harness
        // does not have. The ordered one lives in `alloc`.
        .btree_map(["."])
        .compile_protos(&[proto], &["../../core/proto/"])
}
