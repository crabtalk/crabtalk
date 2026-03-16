# walrus-telegram

Telegram bot gateway for [OpenWalrus](https://github.com/aspect-build/walrus) agents.

Connects a Telegram bot to OpenWalrus agents via the walrus daemon, streaming
AI responses with edit-in-place updates.

## Install

```
cargo install walrus-telegram
```

Or via the OpenWalrus hub:

```
walrus hub install openwalrus/telegram
```

## Usage

```
walrus-telegram serve --daemon /path/to/daemon.sock --config '{"telegram":{"token":"BOT_TOKEN"}}'
```

The bot token is obtained from [@BotFather](https://t.me/BotFather) on Telegram.

## Features

- Streams AI responses with real-time edit-in-place updates
- MarkdownV2 formatting with plain-text fallback
- Per-chat agent selection via `/switch <agent>`
- Hub install/uninstall commands via `/hub install|uninstall <package>`
- Attachment forwarding (images, audio, video, files)
- Automatic session management with error recovery

## License

MIT
