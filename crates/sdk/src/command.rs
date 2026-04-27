//! Slash-command parsing — the canonical set apps recognise from chat input.

/// A parsed slash command. The set is shared across apps (TUI, telegram,
/// wechat); each app maps the variants to its own dispatch.
pub enum Command {
    /// `/clear` — drop the active conversation, start fresh.
    Clear,
    /// `/exit` — leave the app.
    Exit,
    /// `/help` — show available commands.
    Help,
    /// `/resume` — open the conversation picker.
    Resume,
    /// `/<skill>` (or `/<skill> args`) — forward to the daemon for skill resolution.
    /// Carries the full original line, including the leading `/`.
    Forward(String),
}

/// All built-in slash commands, useful for autocompletion.
pub const COMMANDS: &[&str] = &["/clear", "/exit", "/help", "/resume"];

/// Unknown-command hint shown to users.
pub const COMMAND_HINT: &str = "Unknown command.";

/// Parse a chat-input line into a [`Command`]. Returns `None` for non-slash input.
///
/// Unknown slash names map to [`Command::Forward`] — the daemon resolves them
/// against the agent's skill registry.
pub fn parse_command(content: &str) -> Option<Command> {
    let trimmed = content.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let name = trimmed[1..].split_whitespace().next()?;
    let cmd = match name {
        "clear" => Command::Clear,
        "exit" => Command::Exit,
        "help" => Command::Help,
        "resume" => Command::Resume,
        _ => Command::Forward(content.to_owned()),
    };
    Some(cmd)
}
