# walrus-gateway

Platform gateways for OpenWalrus agents. Connects Telegram and Discord bots to
the daemon's agent event loop. Each platform is a separate binary
(`walrus-telegram`, `walrus-discord`) built from this crate.

## Install

```bash
walrus hub install openwalrus/telegram
walrus hub install openwalrus/discord
```

Or build from source:

```bash
cargo install walrus-gateway
```

This installs both `walrus-telegram` and `walrus-discord` binaries.

## Configuration

Installed automatically by `walrus hub install`. Set your bot token in
`walrus.toml`, or configure interactively with `walrus auth`:

### Telegram

```toml
[services.telegram]
kind = "gateway"
crate = "walrus-telegram"
restart = "on_failure"
enabled = true

[services.telegram.config.telegram]
token = "123456:ABC-DEF..."
```

Get a token from [@BotFather](https://t.me/BotFather) on Telegram.

### Discord

```toml
[services.discord]
kind = "gateway"
crate = "walrus-discord"
restart = "on_failure"
enabled = true

[services.discord.config.discord]
token = "your-bot-token"
```

Create a bot in the [Discord Developer Portal](https://discord.com/developers/applications).
Enable the Message Content Intent under Privileged Gateway Intents.

## Bot commands

| Command | Description |
|---------|-------------|
| `/switch <agent>` | Switch the active agent for this chat |

## License

GPL-3.0
