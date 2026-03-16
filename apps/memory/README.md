# walrus-memory

Graph-based memory extension for OpenWalrus agents. Runs as an extension service
managed by the daemon.

## Install

```bash
walrus hub install openwalrus/memory
```

Or build from source:

```bash
cargo install walrus-memory
```

## Configuration

Installed automatically by `walrus hub install`. To customize, edit `walrus.toml`:

```toml
[services.memory]
kind = "extension"
crate = "walrus-memory"
enabled = true
config = { entities = ["project"], relations = ["implements"], connections = 30 }
```

| Config field | Type | Description | Default |
|-------------|------|-------------|---------|
| `entities` | string[] | Additional entity types | `[]` |
| `relations` | string[] | Additional relation types | `[]` |
| `connections` | usize | Default limit for graph traversal | `20` (max 100) |

## Architecture

Storage: LanceDB with three tables — entities, relations, journals. Embeddings
via candle (all-MiniLM-L6-v2, 384-dim). Shared global graph across all agents.

### Agent-facing tool

- **recall** — batch semantic search + 1-hop graph traversal. Agent sees this
  as a regular tool. Automatically called before each agent run (auto-recall).

### Background extraction

After each agent run, the daemon triggers async extraction via the extension's
Infer capability. An extraction LLM reviews the conversation and calls two
internal tools:

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

## Development

Normally launched by the daemon. For standalone development:

```sh
walrus-memory serve --socket /tmp/memory.sock
```

## License

MIT OR Apache-2.0
