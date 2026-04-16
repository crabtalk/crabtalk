# Memory

Memory is a single-file entry store, shared by an agent across its conversations. It holds two kinds of content: notes that the agent writes deliberately, and archives that accumulate as conversations are compacted. Search is lexical (BM25); there are no embeddings.

## Entries

An entry has:

- `id` — monotonic integer, assigned on insert.
- `name` — the entry's primary identifier. Unique within the memory.
- `aliases` — alternative names that resolve to the same entry.
- `content` — the entry's text.
- `kind` — `Note` or `Archive`.
- `created_at` — creation timestamp.

Entries are addressed by `name` or by any of their `aliases`. A name is rebindable through aliasing; the canonical `name` is whatever the agent most recently chose.

## Kinds

`Note` entries are the agent's long-term store. The agent adds, renames, aliases, and rewrites them through memory operations.

`Archive` entries are produced by compaction. Their `content` is the summary of a compacted conversation prefix. Archive entries are not rewritten after creation.

Both kinds share the same index and search path. A search over memory returns both, ranked by relevance.

## Compaction

Compaction compresses a prefix of a conversation's history into a summary and records a boundary in the history at the point of compression.

When a conversation is compacted:

1. The daemon summarizes the history prefix.
2. The summary is written to the memory as an `Archive` entry with a generated `name`.
3. A compact marker is appended to the conversation's history, carrying the `archive_name` and `archived_at` timestamp.

On the next run, the history is replayed from the latest compact marker. Entries before the marker are dropped from the working context; the archive remains available through memory search and by explicit name.

A conversation can be compacted any number of times. Each compaction leaves one additional marker and one additional archive entry.

## Persistence

The memory is a single file. The file holds all entries, all aliases, and the search index snapshot. A write operation mutates memory in RAM and writes an atomic snapshot of the file on each successful apply.

Opening an existing path reads the snapshot into RAM. Opening a non-existent path creates an empty memory; the file is written on the first successful apply.

## Search

Search is BM25 over the tokenized content and name of each entry. Results include the entry and its score. The caller chooses the cutoff — the store does not filter by relevance.

The token set is the union of tokens from `content` and `name`; aliases do not contribute tokens. Aliases are resolution, not search.

## Operations

Memory exposes a closed set of write operations:

| Operation | Effect                                                 |
|-----------|--------------------------------------------------------|
| `Add`     | Create a new entry with a given name, content, and kind. |
| `Rename`  | Change an entry's canonical name.                       |
| `Alias`   | Bind an additional name to an existing entry.           |
| `Write`   | Replace an entry's content.                             |
| `Remove`  | Delete an entry and all its aliases.                    |

Operations on `Archive` entries are permitted but not expected; the agent works with `Note` entries.
