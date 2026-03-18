//! Shared bot command parsing.
//!
//! Platform-agnostic command types and parser. Each platform adapter
//! provides its own `dispatch_command` that uses these shared types.

/// A parsed bot command from a `/cmd` message.
pub enum BotCommand {
    HubInstall { package: String },
    HubUninstall { package: String },
}

/// Unknown command hint shown to users.
pub const COMMAND_HINT: &str =
    "Unknown command. Available: /hub install <pkg>, /hub uninstall <pkg>";

/// Parse a message content string into a `BotCommand`.
///
/// Returns `None` for non-`/` messages or unrecognised commands.
pub fn parse_command(content: &str) -> Option<BotCommand> {
    let mut parts = content.split_whitespace();
    let first = parts.next()?;
    if !first.starts_with('/') {
        return None;
    }

    match first {
        "/hub" => {
            let sub = parts.next()?;
            let arg = parts.next().map(str::to_owned).unwrap_or_default();
            match sub {
                "install" => Some(BotCommand::HubInstall { package: arg }),
                "uninstall" => Some(BotCommand::HubUninstall { package: arg }),
                _ => None,
            }
        }
        _ => None,
    }
}
