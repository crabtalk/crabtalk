# Walrus

[![Crates.io][crates-badge]][crates]
[![Docs][docs-badge]][docs]
[![Discord][discord-badge]][discord]

**The composable agent runtime.** Compact daemon core. Memory, channels,
tools — all hooks. Use what you need, skip what you don't.

```bash
curl -fsSL https://openwalrus.xyz/install.sh | sh
```

Or `cargo install openwalrus`. See the [installation guide][install] for details.

## Quick Start

```bash
# Start the daemon
walrus daemon

# Chat with your agent
walrus attach
```

Point it at any LLM — [Ollama][providers], [OpenAI, Anthropic, DeepSeek][remote], or any OpenAI-compatible API.

```toml
[walrus]
model = "qwen3:4b"

[model.qwen3]
base_url = "http://localhost:11434/v1"
```

Full config reference: [configuration][config].

## How It Works

Walrus is a daemon that runs [agents] and dispatches tools. The daemon
ships with built-in [tools] (file I/O, shell, task delegation),
[MCP][mcp] server integration, and [skills] (Markdown prompt files).

Heavier capabilities live outside the daemon as [extensions][services] —
managed child processes you add or remove in config:

| Service            | What it does                                 |
| ------------------ | -------------------------------------------- |
| [Memory][memory]   | Graph memory — LanceDB + semantic embeddings |
| [Search][search]   | Meta-search aggregator                       |
| [Gateway][gateway] | Telegram, Discord adapters                   |

The daemon stays small. Services scale independently.

## Learn More

- [Quickstart][quickstart] — first agent in 2 minutes
- [Configuration][config] — walrus.toml reference
- [Providers][providers] — connect any LLM
- [Extensions][services] — how extensions work
- [Architecture][runtime] — runtime, event loop, hooks
- [Why we built OpenWalrus][blog]

## License

GPL-3.0

<!-- badges -->

[crates-badge]: https://img.shields.io/crates/v/openwalrus.svg
[crates]: https://crates.io/crates/openwalrus
[docs-badge]: https://img.shields.io/badge/docs-openwalrus.xyz-blue
[docs]: https://openwalrus.xyz/docs/walrus
[discord-badge]: https://img.shields.io/discord/1481168707391852659?label=discord
[discord]: https://discord.gg/XxyxfNX3Fn

<!-- docs -->

[install]: https://openwalrus.xyz/docs/walrus/getting-started/installation
[quickstart]: https://openwalrus.xyz/docs/walrus/getting-started/quickstart
[config]: https://openwalrus.xyz/docs/walrus/getting-started/configuration
[providers]: https://openwalrus.xyz/docs/walrus/models/providers
[remote]: https://openwalrus.xyz/docs/walrus/models/remote
[agents]: https://openwalrus.xyz/docs/development/concepts/agents
[runtime]: https://openwalrus.xyz/docs/development/concepts/runtime
[services]: https://openwalrus.xyz/docs/walrus/extensions
[memory]: https://openwalrus.xyz/docs/walrus/extensions/memory
[search]: https://openwalrus.xyz/docs/walrus/extensions/search
[gateway]: https://openwalrus.xyz/docs/walrus/extensions/gateway
[tools]: https://openwalrus.xyz/docs/development/tools/built-in
[mcp]: https://openwalrus.xyz/docs/development/tools/mcp
[skills]: https://openwalrus.xyz/docs/development/tools/skills
[blog]: https://openwalrus.xyz/blog/why-we-built-openwalrus
