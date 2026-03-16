# walrus-discord

Discord bot gateway for [OpenWalrus](https://github.com/aspect-build/walrus) agents.

Connects a Discord bot to OpenWalrus agents via the walrus daemon.

## Install

```
cargo install walrus-discord
```

Or via the OpenWalrus hub:

```
walrus hub install openwalrus/discord
```

## Usage

```
walrus-discord serve --daemon /path/to/daemon.sock --config '{"discord":{"token":"BOT_TOKEN"}}'
```

The bot token is obtained from the [Discord Developer Portal](https://discord.com/developers/applications).

## Features

- Streams AI responses and sends the final reply on completion
- Per-chat agent selection via `/switch <agent>`
- Hub install/uninstall commands via `/hub install|uninstall <package>`
- Attachment forwarding (images, audio, video, files)
- Automatic session management with error recovery

## License

MIT
