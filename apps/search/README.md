# wsearch

A meta search engine CLI that aggregates results from multiple backends with
consensus-based ranking.

## Features

- **Multi-engine search** — queries DuckDuckGo and Wikipedia in parallel, merges
  and deduplicates results
- **Consensus ranking** — results found by multiple engines get a score boost
- **Web page fetching** — extracts clean text content from any URL, stripping
  scripts, styles, navigation, and other noise
- **In-memory caching** — configurable TTL to avoid redundant queries
- **Multiple output formats** — JSON, plain text, or compact

## Install

```sh
cargo install wsearch
```

## Usage

### Search

```sh
wsearch search "rust programming language"
wsearch search "openwalrus" --engines wikipedia
wsearch search "hello world" -n 5 --format text
```

### Fetch a web page

```sh
wsearch fetch "https://example.com"
wsearch fetch "https://example.com" --format text
```

### List available engines

```sh
wsearch engines
```

### Configuration

Generate a default config file:

```sh
wsearch config --init > ~/.config/wsearch/config.toml
```

Show current configuration:

```sh
wsearch config
```

#### Config file format (TOML)

```toml
engines = ["duckduckgo", "wikipedia"]
timeout_secs = 10
max_results = 20
cache_ttl_secs = 300
output_format = "json"
```

The config file is loaded from `~/.config/wsearch/config.toml` by default, or
specify a path with `--config <path>`.

## Library usage

The crate also exposes a library API:

```rust
use wsearch::aggregator::Aggregator;
use wsearch::config::Config;

let config = Config::default();
let aggregator = Aggregator::new(&config);
let results = aggregator.search("rust", 1).await?;
```

## License

MIT OR Apache-2.0
