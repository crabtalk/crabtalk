//! Prost-generated protobuf types for wire encoding.
//!
//! Base protocol types live in the root of this module. WHS types
//! live in the [`whs_proto`] submodule. Domain re-exports in
//! [`super::message`] and [`super::whs`] provide stable import paths.

include!(concat!(env!("OUT_DIR"), "/walrus.protocol.rs"));

/// WHS (Walrus Hook Service) protocol types — generated from `whs.proto`.
pub mod whs_proto {
    include!(concat!(env!("OUT_DIR"), "/walrus.whs.rs"));
}
