//! Crabtalk plugin management library.
//!
//! Provides manifest parsing and install/uninstall operations for plugins.
//! Designed as a library so any client (CLI, macOS app) can call it directly.

pub mod manifest;
pub mod plugin;
