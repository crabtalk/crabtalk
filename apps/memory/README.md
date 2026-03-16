# walrus-memory

Graph-based memory extension for walrus agents. Runs as an extension service
child process managed by the daemon.

## Architecture

Storage: LanceDB with three tables — entities, relations, journals. Embeddings
via candle (all-MiniLM-L6-v2, 384-dim). Shared global graph across all agents.

### Agent-facing tool

- **recall** — batch semantic search + 1-hop graph traversal. Agent sees this
  as a regular tool. Automatically called before each agent run (auto-recall).

### Background extraction

After each agent run, the daemon triggers async extraction via the WHS Infer
capability. An extraction LLM reviews the conversation and calls two internal
tools:

- **recall** — check existing memories for dedup
- **extract** — batch upsert entities and relations

The agent never calls `extract` directly — it's only used by the extraction LLM.

### Lifecycle hooks

| Hook | Behavior |
|------|----------|
| BuildAgent | Injects identity/profile entities into system prompt |
| BeforeRun | Auto-recall: searches memory, injects `<recall>` block |
| AfterRun | Stores conversation journal, triggers extraction via Infer |
| Compact | Enriches compaction prompt with recent journals |
| ServiceQuery | Exposes entities, relations, journals, search operations |

## Configuration

In `walrus.toml`:

```toml
[services.memory]
kind = "hook"
command = "walrus-memory serve"
config = { auto_recall = true }
```

- **auto_recall** — inject memory context before each agent run (default: true)

## Usage

Normally launched by the daemon. For development:

```sh
walrus-memory serve --socket /tmp/memory.sock
```

## License

MIT OR Apache-2.0
