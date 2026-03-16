# walrus-search

Meta-search extension for OpenWalrus agents. Aggregates results from DuckDuckGo
and Wikipedia with consensus-based ranking. No API keys required.

## Install

```bash
walrus hub install openwalrus/search
```

Or build from source:

```bash
cargo install walrus-search
```

## Configuration

Installed automatically by `walrus hub install`. Default config in `walrus.toml`:

```toml
[services.search]
kind = "extension"
crate = "walrus-search"
enabled = true
```

No additional configuration needed.

## How it works

1. Agent calls the `web_search` tool with a query
2. The service queries DuckDuckGo and Wikipedia in parallel
3. Results are merged, deduplicated, and ranked by cross-engine consensus
4. Top results are returned to the agent as structured data

## Standalone CLI

The binary also works as a standalone search CLI:

```sh
walrus-search search "rust programming language"
walrus-search search "openwalrus" --engines wikipedia
walrus-search search "hello world" -n 5 --format text
walrus-search fetch "https://example.com"
walrus-search engines
```

## License

MIT OR Apache-2.0
