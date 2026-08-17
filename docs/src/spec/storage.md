# Storage

The daemon persists through two mechanisms. The `Storage` trait covers agents,
sessions, and configuration; a binary picks a backend, and the runtime holds it
behind `Config::Storage` without learning which one it got. Two backends ship —
SQLite and a filesystem tree — chosen at startup and not reconsidered on reload.

The memory store is the other, and it is not a `Storage` backend: a single file
of its own, written by the memory hook. Both are in this chapter because the
line between them is currently in the wrong place, and that is easier to see
with both on the page.

## Config, whole

An agent is stored as its `AgentConfig`, serialized whole, with `name` as a
column because lookup by name is a trait method.

Nothing queries *inside* a config, so a column per field would buy no index and
cost a migration every time the struct gains one — and it does gain them
(`mcps` went from `Vec<String>` to full configs; `harnesses` arrived later).
The one exception to that rule used to be `system_prompt`, which had a column of
its own and a matching `prompt` parameter threaded through every write. It is
gone: an agent's `description` **is** its system message, and it serializes with
everything else.

## Sessions

A session is a conversation's persistent form, addressed by a `SessionHandle`
derived from `(agent, sender)`. It holds:

- **Messages** — the `HistoryEntry` stream, one row per entry, appended.
- **Events** — the `EventLine` trace, indexed by kind so a rollup like
  "total token usage for this session" is a query rather than a scan.
- **Meta** — title, timestamps, message count, summary, and the archive pointer.

Writes are appends. `truncate_session_messages` exists for compaction, which is
the only operation that removes history, and `append_session_compact` records
the boundary.

## Migrations

There is no migration table. Every DDL statement is `IF NOT EXISTS`, so opening
an existing database is a no-op and there is nothing to keep in step.

That works for adding tables and indexes and does not work for renaming or
dropping a column — which is a real constraint on schema changes, not an
oversight to route around. A change that needs one either restructures to avoid
it (folding a column into the config blob, so old databases keep a vestigial
column nobody writes) or brings a migration mechanism with it.

## Scaffold

`scaffold` creates the layout and seeds a default agent on first run, so a
fresh install has something to talk to. It is the only write the daemon makes
that the user did not ask for.

## The memory store

Memory is a single-file entry store, shared by an agent across its conversations. It holds what an agent deliberately wrote down. Search is lexical (BM25); there are no embeddings.

### Entries

An entry has:

- `id` — monotonic integer, assigned on insert.
- `name` — the entry's primary identifier. Unique within the memory.
- `aliases` — alternative names that resolve to the same entry.
- `content` — the entry's text.
- `kind` — `Note` or `Archive`.
- `created_at` — creation timestamp.

Entries are addressed by `name` or by any of their `aliases`. A name is rebindable through aliasing; the canonical `name` is whatever the agent most recently chose.

### Kinds

`Note` entries are the agent's long-term store. The agent adds, renames, aliases, and rewrites them through memory operations.

`Archive` entries are produced by compaction. Their `content` is the summary of a compacted conversation prefix. Archive entries are not rewritten after creation.

Both kinds share the same index and search path. A search over memory returns both, ranked by relevance.

### Compaction

Compaction compresses a prefix of a conversation's history into a summary and records a boundary in the history at the point of compression.

When a conversation is compacted:

1. The daemon summarizes the history prefix.
2. The summary is written to the memory as an `Archive` entry with a generated `name`.
3. A compact marker is appended to the conversation's history, carrying the `archive_name` and `archived_at` timestamp.

On the next run, the history is replayed from the latest compact marker. Entries before the marker are dropped from the working context; the archive remains available through memory search and by explicit name.

A conversation can be compacted any number of times. Each compaction leaves one additional marker and one additional archive entry.

### Persistence

The memory is a single file. The file holds all entries, all aliases, and the search index snapshot. A write operation mutates memory in RAM and writes an atomic snapshot of the file on each successful apply.

Opening an existing path reads the snapshot into RAM. Opening a non-existent path creates an empty memory; the file is written on the first successful apply.

### Search

Search is BM25 over the tokenized content and name of each entry. Results include the entry and its score. The caller chooses the cutoff — the store does not filter by relevance.

The token set is the union of tokens from `content` and `name`; aliases do not contribute tokens. Aliases are resolution, not search.

### Operations

Memory exposes a closed set of write operations:

| Operation | Effect                                                 |
|-----------|--------------------------------------------------------|
| `Add`     | Create a new entry with a given name, content, and kind. |
| `Rename`  | Change an entry's canonical name.                       |
| `Alias`   | Bind an additional name to an existing entry.           |
| `Write`   | Replace an entry's content.                             |
| `Remove`  | Delete an entry and all its aliases.                    |

Operations on `Archive` entries are permitted but not expected; the agent works with `Note` entries.

### Surface

The store is a feature, not lifecycle state, so its tools reach agents through a
hook rather than through the runtime. That also makes them available to anyone
embedding the runtime as a library, which a protocol message would not.

## Archives are on the wrong side

A compaction summary is written to the memory store as an `Archive` entry, and
the sessions table holds a pointer to it by name. That is the coupling that
forces `Runtime` to carry a memory handle: compaction and resume are lifetime
operations, so if archives live in memory, the lifetime engine must hold the
store.

They should be session rows. Nothing *chose* to remember a compacted prefix —
it is the conversation, compressed, and it is unreachable except through the
session that points at it. Moving it leaves the memory store holding only what
an agent deliberately wrote down, which is what it is for, and lets memory be
an ordinary hook.
