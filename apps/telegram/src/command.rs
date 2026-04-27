//! Telegram bot command dispatch.

use sdk::BotCommand;
use teloxide::prelude::*;

/// Execute a bot command, streaming progress messages back to the originating chat.
pub async fn dispatch_command(cmd: BotCommand, _bot: Bot, _chat_id: i64) {
    match cmd {}
}
