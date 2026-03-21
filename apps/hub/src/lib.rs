//! Crabtalk hub — package management library.
//!
//! Provides manifest parsing and install/uninstall operations for hub packages.
//! Designed as a library so any client (CLI, macOS app) can call it directly.

pub mod manifest;
pub mod package;
