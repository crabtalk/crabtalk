# Crabtalk

[![Crates.io][crates-badge]][crates]
[![Docs][docs-badge]][docs]
[![Discord][discord-badge]][discord]

**Agent daemon.** Runs agents, dispatches tools, connects to MCP servers.
Start it, talk to it, extend it with plugins.

```bash
curl -fsSL https://crabtalk.ai/install.sh | sh
```

Or `cargo install crabup` and use it to pull the rest. See the [installation guide][install] for details.

## Quick Start

```bash
cargo install crabup         # one-time: install the package manager
crabup pull daemon           # fetch the daemon binary
crabup pull tui              # fetch the TUI client
crabtalkd setup              # one-time interactive LLM endpoint config
crabup daemon start          # install the service unit and start it
crabtalk-tui                 # chat
```

Full config reference: [`crates/crabtalk/config.toml`](crates/crabtalk/config.toml).

## How It Works

The daemon ships with built-in tools (shell, task delegation, memory),
MCP server integration, and skills (Markdown prompt files).

Heavier capabilities live outside the daemon as [services](services/) —
independent binaries that connect via auto-discovery. [Apps](apps/)
are agent-powered experiences built on top of the daemon.

## Learn More

- [The Crabtalk Book][book] — manifesto, architecture, and design RFCs
- [Configuration](crates/crabtalk/config.toml) — config.toml reference
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
