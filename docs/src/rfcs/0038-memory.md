# 0038 - Memory

> **Superseded by [0150 - Memory Store](0150-memory-store.md).**
> The file-per-entry premise was replaced by a single binary db file with a markdown dump tree for humans. `description` is gone; `aliases` and `EntryKind` (Note / Archive / Prompt) are new; the reserved `global` Prompt entry replaces `MEMORY.md`.

- Feature Name: Memory System
- Start Date: 2026-02-10
- Discussion: [#38](https://github.com/crabtalk/crabtalk/issues/38)
- Crates: runtime

## Summary

File-per-entry memory with BM25-ranked recall, a curated index (MEMORY.md), and
an identity file (Crab.md) for agent personality. No database — just files.

## Motivation

Agents need persistent knowledge across sessions. The original approach used a
graph memory backed by a database, but that added operational weight and
complexity for what is fundamentally a collection of text entries that need to
be searched.

The system must:

- Store entries as individual files (inspectable, editable by humans).
- Search by relevance, not just exact match.
- Inject relevant memories automatically before each agent turn.
- Support a curated overview (MEMORY.md) that is always present in context.
- Support an identity/soul file (Crab.md) for agent personality.

## Design

### Directory structure

```
~/.crabtalk/config/
├── Crab.md                  # identity file (one level above memory/)
└── memory/
    ├── entries/
    │   ├── entry-name.md
    │   └── ...
    └── MEMORY.md
```

Crab.md lives one level above `memory/` because it's an agent-level identity
file, not a memory entry. It's shared across the config, not scoped to memory.

### Entry format

Frontmatter markdown. Each entry has a name, description (used for search), and
content.

```markdown
---
name: Entry Name
description: Short searchable description
---

Long-form content here.
```

Filenames are slugified from the entry name: `entry-name.md`.

### Recall pipeline

BM25 scoring over all entries. The query is matched against the concatenation of
description + content. Results are ranked by relevance and capped at
`recall_limit` (configurable).

### Auto-recall

Before each agent turn (`on_before_run`), the system extracts the first 8 words
of the last user message (an arbitrary cutoff — short enough to avoid noise,
long enough to carry intent), runs `recall()`, and injects matching results as
an auto-injected `<recall>` block. Auto-injected messages are not persisted and
are refreshed every turn.

### System prompt injection

- **MEMORY.md** — injected as a `<memory>` block in the system prompt via
  `build_prompt()`. Always present if non-empty.
- **Crab.md** — the identity file. Injected via `build_soul()`. Writing is
  gated by `soul_editable` config.
- **Memory prompt** — instructions for the agent on how to use memory tools,
  included from `prompts/memory.md`.

### Tools

Four tools exposed to agents:

- `remember(name, description, content)` — create or overwrite an entry.
- `forget(name)` — delete an entry.
- `recall(query, limit)` — BM25 search, returns formatted results.
- `memory(content)` — overwrite MEMORY.md index.

## Alternatives

**Graph memory with database.** The original system. Rejected for operational
complexity. Files are simpler, inspectable, and sufficient for the use case.

**Embedding-based search.** Would require a vector store and embedding model.
BM25 is fast, dependency-free, and works well enough for the entry sizes we
deal with.

**Single file storage.** One big memory file instead of file-per-entry. Rejected
because individual files are easier to inspect, edit, and version.

## Unresolved Questions

- Should auto-recall use more than the first 8 words for the query?
- Should entries support tags or categories for non-BM25 filtering?
