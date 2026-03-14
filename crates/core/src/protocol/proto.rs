//! Prost-generated protobuf types for wire encoding.
//!
//! These types are the on-the-wire representation. Domain types in
//! [`super::message`] and [`super::whs`] are converted to/from these
//! at the codec boundary.

include!(concat!(env!("OUT_DIR"), "/walrus.protocol.rs"));
