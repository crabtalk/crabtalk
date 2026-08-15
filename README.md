# Crabtalk

[![Crates.io][crates-badge]][crates]
[![Docs][docs-badge]][docs]
[![Discord][discord-badge]][discord]

**Agent daemon.** Runs agents, dispatches tools, connects to MCP servers.
Start it, talk to it, extend it with packages.

```bash
curl -fsSL https://crabtalk.ai/install.sh | sh
```

Or `cargo install crabup` and use it to fetch the rest. See the [installation guide][install] for details.

## Quick Start

```bash
cargo install crabup         # one-time: install the package manager
crabup install daemon        # fetch the daemon binary
crabup install cli           # fetch the CLI client
crabup daemon start          # install the service unit and start it
crabtalk                     # chat
```

Set `llm.base_url` in `~/.crabtalk/config.toml` before the first chat — the
daemon scaffolds the file on first run and warns if the endpoint is unset.

Harness services attach to a running daemon and use `add` instead:

```bash
crabup add cron              # scheduler
crabup add search            # meta-search
```

Full config reference: [`crates/crabtalk/config.toml`](crates/crabtalk/config.toml).

## How It Works

The daemon ships with built-in tools (shell, task delegation, memory),
MCP server integration, and skills (Markdown prompt files).

[Apps](apps/) are agent-powered experiences and standalone services
built on top of the daemon — independent binaries that connect via
auto-discovery.

## Learn More

- [The Crabtalk Book][book] — manifesto, architecture, and design RFCs
- [Configuration](crates/crabtalk/config.toml) — config.toml reference
- [Contributing](CONTRIBUTING.md) — architecture, layering, and data flow

## License

MIT

<!-- badges -->

[crates-badge]: https://img.shields.io/crates/v/crabtalk.svg
[crates]: https://crates.io/crates/crabtalk
[docs-badge]: https://img.shields.io/badge/docs-crabtalk.ai-blue
[docs]: https://crabtalk.ai/docs/crabtalk
[discord-badge]: https://img.shields.io/discord/1481168707391852659?label=discord
[discord]: https://discord.gg/XxyxfNX3Fn

<!-- docs -->

[book]: https://crabtalk.github.io/crabtalk
[install]: https://crabtalk.ai/docs/crabtalk/installation
