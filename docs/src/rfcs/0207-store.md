# 0207 - Store

- Feature Name: Store
- Start Date: 2026-08-18
- Discussion: [#207](https://github.com/crabtalk/crabtalk/pull/207)
- Crates: store, crabdb, runtime, crabtalk, agent
- Supersedes: [0075 (Hook)](0075-hook.md), [0150 (Memory Store)](0150-memory-store.md), [0185 (Session Search and Storage Primitives)](0185-session-search.md)
- Updates: [0193 (Agent-Owned MCP)](0193-agent-owned-mcp.md), [0205 (Berm)](0205-berm.md)

## Summary

Persistence is one primitive. A store implements five methods — `get`, `put`, `delete`, `scan_keys`, `scan` — and is thereby already an `Agents`, a `Sessions`, a `Memory`, a `Skills`, a `Harnesses` and a `TextSearch`, because each of those traits is bounded on `KVStorage`, carries its own method bodies, and is blanket-implemented for anything satisfying it. There is no wrapper to construct and nothing to wire. Secondary indexes are keys; ranked full-text search is BM25 over the same keyspace; and the shipped backend is `crabdb`, an append-only single-file store with a resident key index, replacing SQLite.

## Motivation

The `Storage` trait this replaces had twenty methods and one implementation per backend, so every backend reimplemented sessions, agents, skills and config from scratch. Splitting it into a KV half and a SQL half did not fix that — the SQL half had twenty domain-named methods of its own (`index_agent`, `latest_session`, `skill_summaries`) against four hand-rolled entity tables holding data the KV half already held. It was the domain model restated one layer down, and a backend author still wrote bespoke SQL per entity. Narrowness is not the same property as primitiveness: a trait can be closed and still be the domain in disguise.

The observation that collapses it is that an ordered lookup, a name resolution and a set membership are all secondary indexes, and a secondary index is just more keys. `find_latest_session` does not need a query planner; it needs keys that sort. Once that is true of everything except ranked full-text, and ranked full-text turns out to be an inverted index — a map from term to documents, which is what a keyspace is — nothing is left that a relational engine was doing.

What remained of SQLite at that point was one table with three columns, no joins, no aggregates and no transactions: a parser and a query planner running on every `get` to perform a B-tree lookup. A runtime crate has no business linking a database, and the cost of one is not paid for by what this design uses of it.

## Design

### One primitive

```rust
pub trait KVStorage: Send + Sync + 'static {
    fn get(&self, col: Column, key: &[u8]) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;
    fn put(&self, col: Column, key: &[u8], value: &[u8]) -> impl Future<Output = Result<()>> + Send;
    fn delete(&self, col: Column, key: &[u8]) -> impl Future<Output = Result<bool>> + Send;
    fn scan_keys(&self, col: Column, prefix: &[u8]) -> impl Future<Output = Result<Vec<Vec<u8>>>> + Send;
    fn scan(&self, col: Column, prefix: &[u8]) -> impl Future<Output = Result<Vec<(Vec<u8>, Vec<u8>)>>> + Send;
}
```

Provided beside them, never overridden except by a multi-realm backend: `realm()`, `key(parts)`, `prefix(parts)`, `get_json`, `put_json`. `Column` is a hard partition — a scan in one never sees another's keys — and it exists so a backend may treat kinds differently if it wants to.

Everything above is blanket-implemented:

```text
KVStorage
  └─ TextSearch: KVStorage            BM25 over keys
       ├─ Agents                      blanket over KVStorage
       ├─ Skills                      blanket over KVStorage
       ├─ Harnesses                   blanket over KVStorage
       ├─ Memory                      blanket over KVStorage + TextSearch
       └─ Sessions                    blanket over KVStorage + TextSearch
Backend = the five, blanket
```

The cost of the blanket impls is that a backend cannot override a default with a native fast path — coherence forbids a specific impl where a blanket one exists. That is accepted: the alternative is one empty `impl` line per trait per backend, and no backend has yet wanted the override.

### The keyspace

```text
Agent    agent/{id}                              AgentConfig
         idx/agent/{name}                        id
Session  session/{handle}/meta                   SessionMeta
         session/{handle}/archive                memory entry name
         session/{handle}/msg/{idx:012}          HistoryEntry
         session/{handle}/evt/{idx:012}          EventLine
         idx/sess/{agent}/{by}/{created_at}/{h}  handle
Memory   memory/{name}                           MemoryEntry
Skill    skill/meta/{name}                       SkillSummary
         skill/body/{name}                       SKILL.md
Harness  image/{digest}                          ELF
         name/{name}                             digest
Config   default_agent                           id
Text     idx/text/{ix}/doc/{key}                 len, weight, terms
         idx/text/{ix}/term/{term}/{key}         term frequency
         idx/text/{ix}/stats                     doc count, total length
```

Ordering is load-bearing rather than incidental. `created_at` is RFC3339 and sorts lexicographically, so `indexed_handles` reads an agent's sessions newest-last with no separate sort step. `agent_ids` reads ids straight out of the name index, already name-sorted, touching no configs. Message indices are zero-padded to twelve digits because keys sort as bytes and `"10"` would otherwise precede `"2"`.

Two shapes appear, each chosen by its dominant access. A session's keys nest under its handle so deleting one is a single prefix sweep. A skill's metadata and body are separate keys so a listing reads names without touching markdown — that property is structural rather than a convention each backend must remember.

Writes are ordered content-first, index-second, so a crash orphans content nothing can reach rather than leaving an index entry pointing at nothing. Every index is rebuildable by scanning content.

### Search

`TextSearch` is four operations that know nothing about what they index: a key, a string, and a number to weight by. A caller wanting a person's own words to outrank a tool's passes a larger weight, and what a "role" is stays in `Sessions` where it belongs.

The index is keys, as above. A document's record names its own terms, which is what makes retraction cheap — dropping a document touches its own postings rather than walking the index. The predecessor in 0150 walked every posting list to delete one entry, so removing a five-hundred-message session was five hundred full-index walks.

A query term ending in `*` prefix-matches, which is free when terms are keys and is the nearest thing to stemming this design offers: the agent writes `deployment process`, later searches `deploy*`, and finds it. A prefix's document frequency is the union it matches, so a broad prefix correctly weighs less than a precise term. Phrase search is deliberately absent — it needs positional postings on every write, and the tokenizer drops stopwords, so a phrase query would be quietly wrong rather than merely unsupported.

What may be indexed at all is decided by `HistoryEntry::indexable`: tool results and tool-call arguments are excluded because both carry credentials often enough that neither belongs in free text a query can reach, and a tool-calling assistant contributes only its function names.

### crabdb

`lib/crabdb` is the shipped store. The format is CRMEM — inherited from 0150, which specified it for memory entries — generalised to opaque keys and values:

```text
header   32 bytes, fixed, rewritable in place
         "CRMEM\0" | version u32 | flags u16 | reserved | index_at u64 | index_len u64
record   op u8 | col u8 | key_len u32 | key | val_len u32 | value
index    count u32 | repeated { col u8 | key_len u32 | key | offset u64 }
```

Records are appended and never edited; the newest record for a key wins. A resident `BTreeMap<(col, key), offset>` makes a lookup one seek and a prefix scan an ordered walk. The map holds offsets rather than values, so residency tracks how many keys exist rather than how much has been written — a four-megabyte harness image costs the same entry as a four-byte posting.

The header is fixed-size and the index is not, so the header holds a pointer and the snapshot lives wherever it last fit. On open the snapshot loads and only the records appended after it are replayed; a crash between checkpoints costs a short tail replay rather than lost writes, and a record torn by a crash ends the replay with the append position reset to the last clean boundary. Compaction rewrites live records to a sibling file and renames, so a crash during compaction costs the work and nothing else.

Durability is honest rather than maximal: writes reach the OS immediately, so a process crash loses nothing, and `fsync` happens on checkpoint and compaction, so a power loss can lose writes since the last one. This is what keeps posting writes cheap, and the text index writes many small records per message.

### Realm

Every key carries a realm prefix. One realm is one store today, so it buys nothing — it is in the format from the first byte so that a backend serving many is a different `KVStorage` impl rather than a key migration, and so that a read outside the realm is inexpressible rather than merely forbidden. The word is deliberately not "tenant": this is a runtime people install, and a solo user is not a tenant of anything.

### Tunables

Ranking numbers are judgements, so they are asked for rather than fixed: `Sessions::config() -> Weights` carries role weights, title and summary boosts, and how many message matches to pull per requested hit; `TextSearch::bm25() -> Bm25` carries `k1` and `b`. Both have defaults, and because both traits are blanket-implemented the defaults are what every store gets today. When one genuinely needs to differ the hook belongs on `TextSearch`, which a backend implements directly and can therefore override.

### What the runtime holds

Nothing derivable. `Runtime` has no agent registry and no memory handle: an agent is read from the store for the run that needs it, built, and dropped. Whether any of it is cached is the backend's decision, which is what makes a different deployment a different implementation rather than a rewrite of the runtime.

The exception is a live session, which holds a steering channel. A channel cannot be persisted, so it is genuinely per-process state and stays.

### The hook lifecycle becomes the harness lifecycle

0075 described a `Hook` trait in `crates/runtime/src/hook.rs`, a `DaemonHook` composite, and a `crates/hooks` crate holding `skill`, `memory`, `mcp`, `os`, `delegate` and `ask_user`. 0205 kept that seam deliberately internal — "internal hooks stay internal Rust" — while making harnesses the *guest* ABI. What it did not anticipate is that the internal seam would take the same name.

`crates/hooks` is deleted. The trait is `Harness` in `crates/runtime/src/harness/`, the composite is `Hooks`, and what used to be hooks are either harnesses proper (`os`, per 0205) or the two that remain internal because they wrap daemon state a guest cannot own (`memory`, `mcp`).

The lifecycle methods change for a reason that belongs to this RFC rather than to 0205. "Registered" is no longer a state an agent can be in — it is in the store or it is not — so `on_register_agent` / `on_unregister_agent` become `on_resolve_agent` / `on_forget_agent`. The first fires per run, for the agent that is running, and **must be idempotent**: there is no registry to call it once. The second fires when an agent is deleted from the store, because that is the only moment nothing will resolve the id again.

That inverts 0075's stated invariant. It promised that by the time `Runtime::agent()` returned, hook state was in place, and that hook state was dropped the moment an agent became invisible — both properties of a registry with a membership boundary. What replaces it is narrower and cheaper: state is in place before the run that needs it, and is proportional to the agents actually working rather than to the agents that exist.

## Alternatives

**A KV primitive plus a SQL index.** Tried and removed. The SQL half became twenty domain-named methods over four entity tables restating KV content, so a backend author still wrote per-entity SQL. A primitive is generic; a closed set of named domain queries is the domain model with a smaller surface.

**A composite type pairing the primitives.** `Store<K, Q>` was built and deleted. It forced a construction step, two generic parameters through every signature, and hand-written `Arc<T>` forwarding impls to satisfy its bounds — all of which vanish when the interfaces are implemented over the primitive directly and auto-deref does the rest.

**A third-party embedded store.** redb and parity-db both satisfy the requirements, and parity-db's native columns match `Column` exactly. Rejected on dependency grounds: the bar here is "better than a directory of files," which is a small enough target to own, and shipping a runtime should not mean shipping someone else's storage engine.

**Keeping SQLite.** It satisfies every requirement, which is why it survived several rounds. What it does not survive is the question of what it is for: one table, three columns, no joins, no transactions.

**An in-memory store with periodic flush.** This is what 0150 did, and it is correct at the scale 0150 sized it for — hundreds to thousands of entries. It does not survive a long-running daemon whose keyspace includes an inverted index over every message ever written, both because RAM grows without bound and because a whole-file flush per write makes indexing a single message quadratic in the store.

**Blobs stored separately from keys.** Considered, since harness images are the only large values and are never scanned. Rejected once values live on disk rather than in RAM: a four-megabyte ELF and a two-hundred-byte agent config then differ only in length.

## Unresolved Questions

- `index_text` writes N posting keys, a document record and a counter update without atomicity. A crash mid-call leaves postings with no record to retract them, and concurrent writers drift the counter. Ranking degrades rather than breaking and the index is rebuildable, but no repair path is written and no `KVStorage::batch` exists.
- Nothing calls `checkpoint()`. Where a daemon loop takes its durability points is undecided.
- Search cost is unmeasured. Each query term is a prefix scan plus one document read per candidate — correct, and appropriate for a personal store, but it has moved from C to Rust and from one query to N reads without a profile.
- Berm still reads harness images from the filesystem. The `Harnesses` interface exists and is unused; wiring it requires `on_resolve_agent` to become async.
