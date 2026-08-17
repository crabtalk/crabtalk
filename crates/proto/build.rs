//! Generate the protocol types once, for whichever world is asking.
//!
//! The daemon and a harness compile the same `crabtalk.proto` (RFC 0205); the
//! only difference is that the host wants serde derives on the way out to JSON
//! and a guest has no serde at all.

use std::io::Result;

fn main() -> Result<()> {
    let proto = "proto/crabtalk.proto";
    println!("cargo:rerun-if-changed={proto}");

    let mut config = prost_build::Config::new();
    // `map` fields default to `std::collections::HashMap`, which a guest does
    // not have. The ordered one lives in `alloc`.
    config.btree_map(["."]);
    if std::env::var_os("CARGO_FEATURE_STD").is_some() {
        config.type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]");
    }
    config.compile_protos(&[proto], &["proto/"])
}
