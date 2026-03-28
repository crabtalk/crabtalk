# Crabtalk

[![Crates.io][crates-badge]][crates]
[![Docs][docs-badge]][docs]
[![Discord][discord-badge]][discord]

**Agent daemon.** Runs agents, dispatches tools, connects to MCP servers.
Start it, talk to it, extend it with components on PATH.

```bash
curl -fsSL https://crabtalk.ai/install.sh | sh
```

Or `cargo install crabtalk`. See the [installation guide][install] for details.

## Quick Start

```bash
# Start the daemon
crabtalk daemon start

# Chat with your agent
crabtalk attach
```

Full config reference: [`crates/daemon/config.toml`](crates/daemon/config.toml).

## How It Works

The daemon ships with built-in tools (shell, task delegation, memory),
MCP server integration, and skills (Markdown prompt files).

Heavier capabilities live outside the daemon as independent binaries.
They connect via auto-discovery (`crabtalk <name>` finds
`crabtalk-<name>` on PATH):

| Component    | What it does                          |
| ------------ | ------------------------------------- |
| Search       | Meta-search aggregator                |
| Telegram     | Telegram gateway                      |
| WeChat       | WeChat gateway                        |
| Outlook      | Outlook MCP server (email + calendar) |
| Hub          | Package management                    |

## Learn More

- [The Crabtalk Book][book] — manifesto, architecture, and design RFCs
- [Configuration](crates/daemon/config.toml) — crab.toml reference
- [Contributing](CONTRIBUTING.md) — architecture, layering, and data flow

## License

MIT OR Apache-2.0

<!-- badges -->

[crates-badge]: https://img.shields.io/crates/v/crabtalk.svg
[crates]: https://crates.io/crates/crabtalk
[docs-badge]: https://img.shields.io/badge/docs-crabtalk.ai-blue
[docs]: https://crabtalk.ai/docs/crabtalk
[discord-badge]: https://img.shields.io/discord/1481168707391852659?label=discord
[discord]: https://discord.gg/XxyxfNX3Fn

<!-- docs -->

[book]: https://crabtalk.github.io/crabtalk
[install]: https://www.crabtalk.ai/docs/getting-started/installation
