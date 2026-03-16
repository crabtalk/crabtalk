//! Prost-generated protobuf types for wire encoding.
//!
//! Base protocol types live in the root of this module. Extension types
//! live in the [`ext_proto`] submodule. Domain re-exports in
//! [`super::message`] and [`super::ext`] provide stable import paths.

include!(concat!(env!("OUT_DIR"), "/walrus.protocol.rs"));

/// Walrus Extension protocol types — generated from `ext.proto`.
pub mod ext_proto {
    include!(concat!(env!("OUT_DIR"), "/walrus.ext.rs"));
}
