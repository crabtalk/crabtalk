# Crabtalk

[![Crates.io][crates-badge]][crates]
[![Docs][docs-badge]][docs]
[![Discord][discord-badge]][discord]

> [!WARNING]
> **Heavy refactor in progress.** The daemon and the CLI are being redesigned
> and are not in the tree; there is no install story until they land. Crate
> layout and APIs are moving with them — treat anything written about
> internals as out of date until this settles.

**Agent daemon.** Runs agents, dispatches tools, connects to MCP servers.
Start it, talk to it, extend it with packages.

```bash
curl -fsSL https://crabtalk.ai/install.sh | sh
```

That installs [`crabup`](apps/crabup), the version manager. See the
[installation guide][install] for details.

Config reference: [`crates/store/src/config/`](crates/store/src/config/).

## How It Works

The daemon owns no tools of its own. Shell, file access, skills and session
search are [harnesses](docs/src/spec/harness.md) an agent declares — one RV64
ELF each, reaching only what that declaration granted. MCP is a capability,
not a harness: calling another program is not shaping an agent.

[Apps](apps/) are agent-powered experiences and standalone services
built on top of the daemon — independent binaries that connect via
auto-discovery.

## Learn More

- [The Crabtalk Book][book] — architecture and design RFCs
- [Architecture](docs/src/arch.md) — harnesses, capabilities, and where a thing goes
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
