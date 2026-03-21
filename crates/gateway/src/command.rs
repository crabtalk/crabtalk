//! Shared bot command parsing.
//!
//! Platform-agnostic command types and parser. Each platform adapter
//! provides its own `dispatch_command` that uses these shared types.

/// A parsed bot command from a `/cmd` message.
pub enum BotCommand {}

/// Unknown command hint shown to users.
pub const COMMAND_HINT: &str = "Unknown command.";

/// Parse a message content string into a `BotCommand`.
///
/// Returns `None` for non-`/` messages or unrecognised commands.
pub fn parse_command(content: &str) -> Option<BotCommand> {
    let first = content.split_whitespace().next()?;
    if !first.starts_with('/') {
        return None;
    }

    None
}
