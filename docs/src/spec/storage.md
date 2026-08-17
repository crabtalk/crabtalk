# Storage

The daemon persists through one primitive. A store implements `KVStorage` — `get`, `put`, `delete`, `scan_keys`, `scan` — and is thereby already an `Agents`, a `Sessions`, a `Memory`, a `Skills`, a `Harnesses` and a `TextSearch`, because each of those traits is bounded on `KVStorage`, carries its own method bodies, and is blanket-implemented for anything satisfying it.

There is nothing to construct and nothing to wire. The runtime names one bound, `Config::Storage: Backend`, and never learns which store it got.

```text
KVStorage                          five methods, the only thing implemented
  └─ TextSearch: KVStorage         BM25 over the keyspace
       ├─ Agents                   blanket over KVStorage
       ├─ Skills                   blanket over KVStorage
       ├─ Harnesses                blanket over KVStorage
       ├─ Memory                   blanket over KVStorage + TextSearch
       └─ Sessions                 blanket over KVStorage + TextSearch
```

`crates/store` links no database and no search crate. Which store to run is a deployment decision, so it lives in the application: `apps/agent` is that wiring, and it is five methods over `lib/crabdb`.

## The keyspace

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

`Column` — the left-hand names — is a hard partition: a scan in one never sees another's keys, and a backend may store one differently from the rest if it wants to.

Every key opens with a realm. One realm is one store today, so it buys nothing; it is in the format from the first byte so a backend serving many is a different `KVStorage` implementation rather than a key migration, and so a read outside the realm is inexpressible rather than merely forbidden.

## Indexes are keys

An ordered lookup, a name resolution, a set membership — all of them are secondary indexes, and a secondary index is just more keys. Nothing here needs a query planner.

`created_at` is RFC3339 and sorts lexicographically, so the newest session for an identity is the last key under its prefix: `find_latest_session` is a prefix scan and a `.last()`. `agent_ids` reads ids straight out of the name index, already sorted by name, without opening a single config. Message indices are zero-padded to twelve digits because keys sort as bytes, and `"10"` would otherwise come before `"2"`.

Two key shapes appear, each chosen by its dominant access. A session's keys nest under its handle, so deleting one is a single prefix sweep. A skill's identity and its body are separate keys, so a listing reads names without touching markdown — a property of the layout rather than a rule each backend has to remember.

## Config, whole

An agent is stored as its `AgentConfig`, serialized whole, with a separate `idx/agent/{name}` key pointing at its id.

Nothing queries *inside* a config, so a field-per-column layout would buy no index and cost a migration every time the struct gains one — and it does gain them (`mcps` went from `Vec<String>` to full configs; `harnesses` arrived later). The name index exists because a person types names; everything else addresses an agent by id, which is why renaming one moves nothing but a label.

The install's own `config.toml` is not in the store. It is hand-written and read from disk on every reload. The one value the daemon decides rather than reads — which agent is default — is store state under `Config`, because a field a program rewrites inside a file a person owns is two sources for one value.

## Sessions

A session is a conversation's persistent form, addressed by an opaque `SessionHandle`. The handle encodes nothing — not the agent, not the sender, not a date — so renaming an agent never orphans its transcripts.

- **Messages** — the `HistoryEntry` stream, one key per entry, appended.
- **Events** — the `EventLine` trace.
- **Meta** — title, timestamps, message count, summary.
- **Archive** — a pointer to the memory entry holding a compacted prefix. The marker carries the pointer; the summary text lives in memory, never beside the session.

Writes are appends. `truncate_session_messages` is the only operation that removes history, and `append_session_compact` records the boundary.

## Search

Ranked full-text is the one lookup keys cannot answer, so it is the one thing built on top — though its index is keys too, since an inverted index is a map from term to documents and a map is what a keyspace is.

`TextSearch` is four operations that know nothing about what they index: a key, a string, and a number to weight by. Whoever wants a person's own words to outrank a tool's passes a larger weight; what a "role" is stays in `Sessions`.

A document's record names its own terms, so retracting one touches its own postings rather than walking the index. A query term ending in `*` prefix-matches — free, when terms are keys — and is the nearest thing to stemming on offer: `deploy*` finds "deployment" and "deployed" where `deploy` finds neither. Phrase search is deliberately absent; it would need positional postings on every write, and the tokenizer drops stopwords, so a phrase query would be quietly wrong rather than unsupported.

What may be indexed at all is decided by `HistoryEntry::indexable`. Tool results and tool-call arguments are excluded, because both carry credentials often enough that neither belongs in free text a query can reach; a tool-calling assistant contributes only its function names.

## crabdb

`lib/crabdb` is the shipped store: an append-only single file, no dependencies, and a bar of "better than a directory of files" rather than "beats a database."

```text
header   32 bytes, fixed, rewritable in place
         "CRMEM\0" | version | flags | reserved | index_at | index_len
record   op | col | key_len | key | val_len | value
index    count | repeated { col | key_len | key | offset }
```

Records are appended and never edited; the newest record for a key wins. A resident `BTreeMap<(col, key), offset>` makes a lookup one seek and a prefix scan an ordered walk. The map holds offsets rather than values, so residency tracks how many keys exist rather than how much has been written — a four-megabyte harness image costs the same entry as a four-byte posting.

The header is fixed and the index is not, so the header holds a pointer and the snapshot sits wherever it last fit. On open the snapshot loads and only records appended after it are replayed. A record torn by a crash ends the replay, with the append position reset to the last clean boundary so the fragment is overwritten. Compaction rewrites live records to a sibling file and renames, so a crash during compaction costs the work and nothing else.

Durability is stated rather than assumed: writes reach the OS immediately, so a process crash loses nothing; `fsync` happens on checkpoint and compaction, so a power loss can lose writes since the last one. That is what keeps posting writes cheap, and the text index writes many small records per message.

## Tuning

Ranking numbers are judgements, so the store is asked for them rather than having them fixed. `Sessions::config() -> Weights` carries role weights, title and summary boosts, and how many message matches to pull per requested hit. `TextSearch::bm25() -> Bm25` carries `k1` and `b`. Both have defaults, and because both traits are blanket-implemented the defaults are what every store gets today.

See [RFC 0207](../rfcs/0207-store.md) for the design and the alternatives it rejected.
