# crabtalk-search

Meta-search component for Crabtalk agents. Aggregates results from DuckDuckGo
and Wikipedia with consensus-based ranking. No API keys required.

## Install

```bash
crabup pull search           # fetch the binary from crates.io
crabup search start          # install and load the service unit
```

Or drive cargo directly if you don't want crabup:

```bash
cargo install crabtalk-search
```

## Configuration

Default config in `crab.toml`:

```toml
[services.search]
kind = "extension"
crate = "crabtalk-search"
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
crabtalk-search search "rust programming language"
crabtalk-search search "crabtalk" --engines wikipedia
crabtalk-search search "hello world" -n 5 --format text
crabtalk-search fetch "https://example.com"
crabtalk-search engines
```

## License

MIT OR Apache-2.0
