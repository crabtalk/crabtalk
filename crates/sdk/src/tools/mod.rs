//! Built-in tool implementations reusable across crabtalk hosts.
//!
//! Today: OS tools (bash, read, edit) packaged as a [`runtime::Hook`].
//! Hosts that wire these in get filesystem + process execution capabilities
//! without re-deriving the schemas or handler logic.

pub mod os;
