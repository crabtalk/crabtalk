//! Walrus CLI — thin client for walrusd. Connects via Unix domain socket.

pub use cmd::{Cli, Command};

pub mod cmd;
pub mod repl;
pub mod tui;
