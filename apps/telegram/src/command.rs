//! Telegram slash-command dispatch.

use sdk::Command;
use teloxide::prelude::*;

/// Execute a slash command on Telegram.
///
/// Most variants are no-ops here — the daemon owns conversation state, so
/// `Clear`/`Resume` don't have a meaningful counterpart on a chat platform
/// that's a single user-bot session per chat. `Help` and `Forward` are
/// handled by the surrounding loop today; this dispatcher exists so future
/// per-platform behaviour has a hook.
pub async fn dispatch_command(_cmd: Command, _bot: Bot, _chat_id: i64) {}
