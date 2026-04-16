# 0150 - Memory Store

- Feature Name: Memory Store
- Start Date: 2026-04-14
- Discussion: [#38](https://github.com/crabtalk/crabtalk/issues/38)
- Crates: memory, crabtalk, runtime
- Supersedes: [0038 (Memory)](0038-memory.md)

## Summary

A standalone `crabtalk-memory` crate backing agent memory with a single binary db file, atomic persistence, and BM25 recall. The markdown tree is a human-facing export — not the primary store. Entries come in two kinds: `Note` (agent-written via `remember`/`forget`) and `Archive` (compaction output). The agent's system prompt is human-managed via `Crab.md` (existing layered-instructions mechanism) — the memory store has no opinion on it.

## Motivation

RFC 0038 bet on file-per-entry markdown as the primary store. In practice that premise did not hold:

- **Atomic writes don't compose across many files.** Every remember/forget touched an entry file plus a sidecar index; a crash mid-op left the tree inconsistent. A single-file db is atomic by rename+fsync.
- **Compaction archives need a store.** Agent-First ([0135](0135-agent-first.md)) made compaction archives first-class long-term memory. Archives share recall and lifecycle with notes, but aren't user-editable text — they're generated output. A kind-typed entry in the db is the right home.
- **Aliases improve recall.** Humans reach for an entry under several names ("release" / "ship" / "deploy"). BM25 needs them as indexable terms, which frontmatter had no slot for.
- **Dump/load still matters for humans.** Users want to read and edit memory with a text editor or mdbook. That's solved by exporting the db as a markdown tree on demand, not by making the tree the source of truth.

A separate observation that shaped the API surface:

- **The system prompt is not memory.** 0038 carried a `MEMORY.md` curated overview that the agent could rewrite via a dedicated `memory` tool. That conflated two different things: persistent recall (the agent's notes) and instructions (the human's prompt). It also gave the agent a footgun — overwriting the whole thing in a single tool call with no diff. Killed: the `memory` tool, the `Prompt` entry kind, and the reserved `global` name. The system prompt now lives in `Crab.md` (already a file, already layered, already human-edited). If a human wants the agent to edit it, they grant that in prose inside `Crab.md` and the agent uses the standard file-edit tools.

## Design

### Crate layout

`crabtalk-memory` is a standalone crate. The `crabtalk` hooks own one `Memory` handle and expose a `SharedStore = Arc<RwLock<Memory>>` to the runtime so compaction can write archives and session resume can read them.

### Binary file format (CRMEM v1)

All integers are little-endian. Strings are UTF-8, length-prefixed by a `u32` byte count (no NUL terminator). The whole file is one contiguous blob — no sections, no index, no padding.

**Header — 16 bytes:**

```text
offset  size  field      value
------  ----  ---------  -------------------------------------------------
 0       6    magic      "CRMEM\0"
 6       4    version    u32  (= 1)
10       2    flags      u16  (= 0; unknown bits rejected on read)
12       4    reserved   [u8; 4] (= 0)
```

**Body:**

```text
size  field        notes
----  -----------  -----------------------------------------------------
 8    next_id      u64   monotonic EntryId allocator; persisted so
                         IDs stay stable across open/close
 4    entry_count  u32
 *    entries      entry_count repetitions of the per-entry record
```

**Per entry:**

```text
size  field        notes
----  -----------  -----------------------------------------------------
 8    id           u64
 8    created_at   u64   unix seconds
 4    kind         u32   0 = Note, 1 = Archive
 4    name_len     u32
 *    name         utf8 bytes, name_len long
 4    content_len  u32
 *    content      utf8 bytes, content_len long
 4    alias_count  u32
 *    aliases      alias_count repetitions of { u32 len + utf8 bytes }
```

`kind` is u32 rather than u8 so the fixed entry prefix stays 4-byte aligned — cheap hygiene for any future on-disk index work. The inverted BM25 index is **not** persisted; it's rebuilt from entries on load. Keeps the file small and the format boring.

**Reader invariants:** magic mismatch, wrong version, non-zero flags, truncated body, invalid UTF-8, or an unknown `kind` value all fail the open with `BadFormat`. A missing file opens an empty db (the file is created on the first successful write).

### Persistence

Every `apply(Op)` mutates RAM then flushes atomically. The flush sequence is:

1. Encode the entire db to an in-memory `Vec<u8>`.
2. `create_dir_all(parent)` if needed.
3. Write to a sibling temp file `{name}.tmp` and `fsync` it.
4. `rename(tmp, path)` — atomic on POSIX when on the same filesystem.
5. `fsync` the parent directory so the rename itself is durable.

A flush failure leaves RAM ahead of disk until the next successful op or the next `open` (which re-reads the file). WAL closes that window in v2. `Memory::checkpoint()` forces the same flush without a mutation.

### Entry model

```rust
enum EntryKind { Note, Archive }

struct Entry {
    id: u64,
    name: String,
    content: String,
    aliases: Vec<String>,
    created_at: u64,
    kind: EntryKind,
}
```

- **Note** — remember/forget entries.
- **Archive** — compaction output. Written by the runtime during compaction, surfaced by `recall` as long-term memory (per 0135).

Kind is immutable per entry: `Update` rewrites content and aliases but keeps kind; use `Remove` + `Add` to change it.

### Write ops

Writes go through an `Op` enum:

```rust
enum Op {
    Add    { name, content, aliases, kind },
    Update { name, content, aliases },
    Alias  { name, aliases },
    Remove { name },
}
```

`Memory::apply(op)` mutates + flushes. Callers never touch `fs::write` directly.

### Recall

BM25 with Lucene-style IDF (`ln((n - df + 0.5)/(df + 0.5) + 1.0)`), k1=1.2, b=0.75. The index is an inverted index of tokens from entry content and aliases, keyed by `EntryId`. Search walks the posting lists for query terms instead of rescanning every entry on every query.

### Auto-recall

Before each agent turn (`before_run`), the hook takes the first 8 words of the last user message, runs BM25 search, and injects hits as an auto-injected `<recall>` user turn. Auto-injected messages are not persisted and refresh every turn.

### System prompt

The hook contributes one `<system_prompt>` fragment: the contents of `prompts/memory.md`, which tells the agent *when* to use the memory tools (tool *signatures* come from each input struct's `///` doc comment via schemars). The agent's identity / behavior prompt is **not** the memory store's responsibility — it's `Crab.md`, layered from `<config_dir>/Crab.md` and any project-local `Crab.md` walked up from CWD (see `daemon::host::discover_instructions`).

### Tools

Three tools exposed to the agent:

- `remember(name, content, aliases)` — upsert a `Note`.
- `forget(name)` — delete a `Note`.
- `recall(query, limit)` — BM25 search, returns formatted results.

There is no `memory` tool. Editing the agent's system prompt is a human action against `Crab.md`. If the human wants to delegate that authority, they say so in `Crab.md` and the agent uses the standard file-edit tools — no special-case tool, no reserved entry name, no parallel write path.

### Dump / load

`Memory::dump(dir)` writes the db as an mdbook-ready tree for humans:

```text
brain/
  book.toml               ← seeded on first dump; user edits survive re-dumps
  SUMMARY.md              ← mdbook ToC (ignored on load)
  notes/{name}.md
  archives/{name}.md
```

The seeded `book.toml` sets `src = "."` so `mdbook serve brain/` works against the tree as-is — no shuffling into an `src/` subdirectory. It's only written when absent; any customizations survive later dumps.

Each entry file starts with an HTML metadata block, followed by pure markdown content:

```markdown
<div id="meta">
<dl>
  <dt>Created</dt>
  <dd><time datetime="2026-04-14T10:23:45Z">2026-04-14T10:23:45Z</time></dd>
  <dt>Aliases</dt>
  <dd><ul><li>ship</li><li>release</li></ul></dd>
</dl>
</div>

prod rollout steps ...
```

Chosen for mdbook: `<dl>` / `<dt>` / `<dd>` is the semantic HTML for key-value metadata, renders as a labeled info card, and doesn't pollute mdbook's heading tree. `<time datetime="...">` round-trips the exact unix timestamp. A file that doesn't start with `<div id="meta">` is treated as pure content with no metadata.

`Memory::load(dir)` reads the tree and *replaces* the db. It validates fully before mutating — a mid-load error leaves the current state untouched. Each kind's subdirectory is cleared on `dump` so renames and deletes don't leave orphan files behind; anything else in `dir` (e.g. a customized `book.toml`, a `theme/` directory) is left alone.

## Alternatives

**Stay with file-per-entry (0038).** Rejected — compaction archives need a real store, and atomic multi-file writes would require WAL anyway. A single file gets atomicity for free.

**SQLite.** Overkill for 10²–10³ entries, adds a dependency and schema migrations. A 200-line hand-rolled format is simpler and easier to inspect with `xxd`.

**Embedding-based search.** Still rejected for the same reasons as 0038: requires a vector store and embedding model. BM25 is fast, dependency-free, and works well at the entry sizes agents produce.

## Unresolved Questions

- WAL for crash safety in the window between the RAM mutation and the atomic flush.
- Should `load()` merge instead of replace?
- Should archives expire or be garbage-collected past some age / count?
