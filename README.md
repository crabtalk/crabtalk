# Walrus

[![Crates.io](https://img.shields.io/crates/v/openwalrus.svg)](https://crates.io/crates/openwalrus)

**The composable agent runtime.** Compact daemon core. Memory, channels,
tools — all hooks. Use what you need, skip what you don't.

```bash
curl -fsSL https://openwalrus.xyz/install.sh | sh
```

Or install with Cargo:

```bash
cargo install openwalrus
```

## Quick Start

```bash
# Start the daemon
walrus daemon

# Chat with your agent
walrus attach
```

Configure in `~/.openwalrus/walrus.toml`:

```toml
[walrus]
model = "qwen3"

# Ollama (no API key needed)
[model.qwen3]
base_url = "http://localhost:11434/v1"

# Or a cloud provider
[model.deepseek-chat]
api_key = "${DEEPSEEK_API_KEY}"
```

## How It Works

Walrus is a daemon that runs agents and dispatches tools. That's the core —
everything else plugs in via **Walrus Hook Services (WHS)**.

| Capability | How it works |
|---|---|
| Memory | WHS — graph memory with LanceDB + semantic embeddings |
| Search | WHS — meta-search aggregator (DuckDuckGo, Wikipedia) |
| Channels | Gateway adapters — Telegram, Discord |
| Tools | Built-in file I/O, shell, MCP servers, task delegation |
| Skills | Markdown prompt files — no code needed |

Services are managed child processes. Add them in config, remove what you
don't need. The daemon stays small.

```toml
[services.memory]
command = "wmemory"

[services.search]
command = "wsearch"
```

## Providers

Any OpenAI-compatible API or Anthropic. Ollama, vLLM, OpenAI, DeepSeek,
Grok — any model name works.

```toml
# OpenAI
[model.gpt-4o]
api_key = "${OPENAI_API_KEY}"

# Anthropic
[model.claude-sonnet-4-20250514]
api_key = "${ANTHROPIC_API_KEY}"
standard = "anthropic"

# Self-hosted
[model.local-llama]
base_url = "http://localhost:8000/v1"
```

## License

GPL-3.0
