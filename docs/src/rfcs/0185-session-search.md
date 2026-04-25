# 0185 - Session Search and Storage Primitives

- Feature Name: Session Search and Storage Primitives
- Start Date: 2026-04-25
- Discussion: [#185](https://github.com/crabtalk/crabtalk/issues/185)
- Crates: core, memory, runtime, crabtalk
- Supersedes: [0171 (Topic Switching)](0171-topic-switching.md)
- Updates: [0135 (Agent-First)](0135-agent-first.md), [0150 (Memory Store)](0150-memory-store.md)

## Summary

Collapse the topic subsystem. Sessions persist unconditionally, always get auto-titled, and carry a small runtime-managed meta blob. Recall gains a second BM25 index — this one over conversation messages — returning windowed excerpts with bounded size. The runtime exposes narrow session primitives and two search tools; client UX owns `/clear`, `/new`, `/compact`, and session routing. The "topic" concept dissolves: content-derived session search (BM25) replaces tag-based grouping, and any curated grouping that survives is a client concern.

## Motivation

RFC 0171 introduced topic switching to partition a single `(agent, sender)` pair into N parallel threads keyed by title, with tmp chats that skip persistence until the agent "promotes" them by entering a topic. In practice it conflated four independent concerns into one knot — routing ("which conversation does this message land in?"), persistence policy ("should this session hit storage?"), recall indexing ("how do we find related past work?"), and lifecycle UX ("when does a chat end and a new one begin?"). Each wanted a different home, and riding one mechanism for all of them produced the `TopicRouter` reservation/rollback dance, the tmp/promote split, and agent-upfront title commitment on what should have been retrospective categorization.

The reframe driving this RFC: *a topic is not a thing*. It was a name trying to be a routing key, a memory kind, a session tag, and a recall index simultaneously. With BM25 over session messages, content-derived recall eats the tag's lunch — the agent searches "cron refactor" and gets back the conversations that actually discussed it, without any of them ever being classified upfront. What remains worth keeping is a `summary` field that boosts search ranking when one happens to exist (piggybacking on work the runtime already does during overflow compaction).

## Design

### Layering: runtime vs. client

The runtime's job is to provide mechanical primitives. UX decisions (when to clear, when to compact for cleanliness, which session to route a message to, how to surface archival browsing) belong one layer up in the client. The one exception is overflow compaction — a context-window overflow is a hard constraint the client can't see coming, so the runtime keeps automatic compaction on overflow as a safety net. Every other form of compaction, clearing, and switching is a client concern composed from primitives.

Runtime primitives (policy-free):

- `new_session(agent, sender) -> id` — always creates, always persists. No tmp, no deferred-persistence gate.
- `append_message(id, msg)` — writes to storage and incrementally updates the session BM25 index.
- `list_sessions(filters?) -> [SessionSummary]` — meta rows only, paginated.
- `list_messages(id, offset, limit) -> [Message]` — paginated browse for when a caller wants to walk a session linearly.
- `get_session_meta(id) -> ConversationMeta` — cheap lookup of current meta snapshot.

Search tools (agent-facing):

- `search_memory(query) -> [Entry]` — unchanged. BM25 over memory entries; returns whole entries because entries are small.
- `search_sessions(query, context_before=4, context_after=4, filters?) -> [SessionHit]` — new. BM25 over message text; returns bounded windowed excerpts.

Auto-behaviors (runtime-owned, mechanical):

- Auto-title generation after the first exchange (unchanged; already in `spawn_title_generation`).
- Overflow compaction under context-window pressure, piggybacking to emit a `summary` into `ConversationMeta` so session search can boost it.

Client-owned (explicit non-goals for the runtime):

- `/clear`, `/new`, `/compact`, "resume session by title", session picker UX — composed from the primitives above.
- Saved searches, archival browsing, "wiki view" — pure presentation.
- Routing decisions — the client tells the runtime *which* `session_id` to append to; the runtime does not infer this from topic state.

### ConversationMeta

The target shape, replacing the current struct in `crates/core/src/storage.rs`:

```rust
pub struct ConversationMeta {
    pub agent: String,            // immutable, set at creation
    pub created_by: String,       // immutable, set at creation
    pub created_at: String,       // immutable, set at creation
    pub title: String,            // auto-generated after first exchange
    pub updated_at: String,       // bumped on every append_message
    pub message_count: u64,       // bumped on every append_message
    pub summary: Option<String>,  // emitted by overflow compaction
}
```

Removed: `topic` (subsumed by session search), `uptime_secs` (replaced by `updated_at`; uptime is derivable if a caller still needs it).

Writers:

| Field                               | Writer          | When                          |
| ----------------------------------- | --------------- | ----------------------------- |
| `agent`, `created_by`, `created_at` | runtime         | session creation              |
| `title`                             | runtime         | auto-gen after 2+ messages    |
| `updated_at`, `message_count`       | runtime         | every `append_message`        |
| `summary`                           | runtime         | during overflow compaction    |

Meta is not an agent-writable blob. The runtime owns every field. If a later RFC needs an agent-curated field (e.g., session-to-entry back-links to optimize resume hydration), it lands as a separate proposal with a measured recall-failure case justifying the code cost — not speculatively in this one.

### Schema migration

Zero-touch upgrade. All meta fields added by this RFC use `#[serde(default)]`; removed fields (`topic`, `uptime_secs`) are silently ignored on deserialize. On the next meta rewrite for a given session (any `append_message` triggers one), the removed fields are dropped from disk. No migration pass, no version bump, no operator intervention. Old session JSONL files mix cleanly with new writes.

Serde config on `ConversationMeta`:
- `#[serde(default)]` on `updated_at`, `message_count`, `summary`.
- `#[serde(default, skip_serializing)]` on the removed fields during the transition window if a `Deserialize` derive would otherwise reject unknown keys — standard `#[serde(default)]` struct-level behavior covers this without explicit `skip`.
- No `deny_unknown_fields` anywhere on this struct.

### Session search — BM25 over messages

The memory crate already ships a 157-line hand-rolled inverted BM25 index (`crates/memory/src/bm25.rs`, zero external deps). Session search reuses this primitive. Two choices, to be decided during implementation: (a) lift `bm25::Index` into a shared module used by both the memory crate and a new session index, or (b) instantiate a parallel index owned by the runtime. Either way, no new workspace deps.

Field weights, inherited from the community Claude Code conversation-search pattern (alexop.dev, `raine/claude-history`):

- `summary` — 3.0× (when present; skipped when absent)
- `title` — 2.0×
- user messages — 1.5×
- assistant messages — 1.0×
- tool-use turns — 1.3× (proxy for "a solution was applied")

**Hit shape with explicit bounds.** Messages can contain large tool results, blobs, or attachments. Returning raw `Message` objects in search windows would defeat the bounding the windowing was meant to provide. The hit type projects to a fixed small shape, not full messages:

```rust
pub struct SessionHit {
    pub session_id: u64,
    pub msg_idx: usize,
    pub score: f64,
    pub meta: SessionSummary,              // title, created_at, updated_at, message_count
    pub window: Vec<WindowItem>,           // context_before + match + context_after
}

pub struct WindowItem {
    pub role: Role,
    pub msg_idx: usize,
    pub snippet: String,                   // truncated to MAX_SNIPPET_BYTES
    pub truncated: bool,
    pub tool_name: Option<String>,         // for tool-use turns
}
```

Hard limits:
- `MAX_SNIPPET_BYTES = 1024` per window item.
- `MAX_WINDOW_ITEMS = context_before + 1 + context_after`, capped at 16 regardless of caller request.
- `MAX_HITS_PER_QUERY = 20`.

A full-message read always goes through `list_messages(session_id, offset, limit)` — there is no "load entire session" primitive, by design.

### Performance budget and cold-start

Concrete targets this RFC commits to:

- **`search_sessions` query latency**: p99 ≤ 50ms at 100k indexed messages; p99 ≤ 200ms at 1M. CPU-only — the index is in memory.
- **`append_message` indexing overhead**: ≤ 1ms added per append at any index size up to 1M messages. Pure CPU.
- **Cold-start index rebuild**: dominated by storage I/O, not BM25. The CPU portion is sub-second at 100k messages, but a real `FsStorage` rebuild does one `load_session` per persisted session — at 100k messages spread across 2k sessions, end-to-end rebuild is on the order of 10–20 seconds on local SSD. **Rebuild runs in the background after daemon startup; live appends index immediately, so new work is always findable. Old sessions become searchable as the rebuild progresses.** A future RFC can add on-disk index checkpointing if cold-rebuild latency becomes a felt operational concern.

These targets are verified by a `criterion` bench against `FsStorage` rooted in a tmpdir, not against the in-memory index alone. Failure of a CPU-side target blocks the phase; storage-bound rebuild time is monitored, not gated.

### Session lifetime and deletion

This RFC treats sessions as immortal. There is no runtime `delete_session` primitive; storage grows unboundedly with agent activity. This is an explicit scope decision: garbage collection is a separate operational concern (retention policy, archival, export-and-prune) that warrants its own RFC once usage patterns reveal what the right policy is. In the meantime, operators who need to prune can do so at the filesystem layer — JSONL files in `sessions/` are safe to delete offline; the index rebuilds from disk on next start.

When delete support lands, it needs to: (a) remove JSONL file, (b) remove postings from the BM25 index, (c) invalidate any in-memory `SessionSummary` cache. None of that is in scope here.

### Auto-compaction as safety net

Overflow compaction stays, because context-window overflow is a hard constraint the client layer can't enforce. Two changes versus today: (a) compaction additionally populates `ConversationMeta.summary` so session search can boost it, and (b) compaction is no longer per-topic (there are no topics) — it fires per session, which is what a client would expect anyway.

The existing `AgentConfig::compact_threshold` continues to fire on token-budget pressure, not overflow-only; "overflow safety net" here is shorthand for "context-pressure-driven, not user-driven." Discretionary compaction ("I want to clean up this old chat") is a client concern — the runtime optionally exposes a `compact(session_id)` helper in a follow-up RFC if clients converge on needing one. Not required to ship this one.

## Alternatives

**Semantic retrieval via embeddings.** Deferred. Lexical BM25 covers the 80% case at zero new deps and microsecond query time. A vector index adds an embedding model or API dependency, hundreds of MB of index storage, and a hybrid-search ranking story. Revisit when lexical recall demonstrably misses on a labeled test set — not before.

**Keep topic as a tag.** Rejected. With BM25 over messages, tag-based filtering is redundant with query-based retrieval at the cost of requiring disciplined agent tagging and introducing tag-name drift ("cron refactor" vs "cron cleanup"). The tag was the join key between memory and sessions; BM25 is the join key now.

**Single unified `recall()` tool that queries memory and sessions together.** Rejected. Two explicit tools are cheaper for the agent to reason about — it knows what it is paying for in each call, and the two stores have different payload-sizing rules (memory entries are small and returned whole; session hits are bounded excerpts). Composition in prompt-space is the right layer.

**Agent-curated session-to-entry back-links (`linked_entries`).** Considered and removed from this RFC. The primitive has a reference-rot problem (entry names change or are deleted; the link silently dangles) and its concrete benefit is a recall optimization whose cost — two new tools, a persisted `Vec<String>`, and a new agent behavior — isn't justified until BM25 demonstrably misses a case it would have caught. If such a case shows up in practice, a follow-up RFC can propose it with reference-by-id semantics and a measured justification.

**Keep `read_session(id)` as full-history load.** Rejected. Unbounded reads are a context-window hazard and the functionality is better served by `list_messages` (paginated browse) plus windowed excerpts from search.

## Migration

Phased implementation, one commit per phase per `CLAUDE.md`'s workflow rule. Order is deliberate: **delete first, build on a clean foundation, then layer the search feature.** This avoids the awkward intermediate state where the topic subsystem and the new primitives coexist.

**Phase 1 — Delete the topic subsystem.** Remove `switch_topic`, `search_topics`, `TopicRouter`, the tmp/promote gating, the entire `crates/crabtalk/src/hooks/topic/` module, `Runtime::switch_topic` and its helpers, and `ConversationMeta.topic` (storage-side). Sessions now always persist. `EntryKind::Topic` is kept for now as a presentation label (see open questions). Commit should be heavily negative line-count — mostly subtraction.

Rollback: `git revert`. Every phase is one commit; revert is the rollback plan.

**Phase 2 — ConversationMeta cleanup.** Drop `uptime_secs`. Add `updated_at` and `message_count`, wired into `append_message`. Verify zero-touch read of existing session files via `serde(default)`. Add nextest coverage for mixed-version reads.

**Phase 3 — Session BM25 index + `search_sessions` tool.** New index in the runtime (decide lift-vs-parallel with memory crate's `bm25::Index` inside this phase). Incremental updates on `append_message`. New tool wired through the hook registry. Add a `criterion` bench verifying the performance budget (§ Performance budget and cold-start). If cold-start rebuild exceeds 500ms at 100k messages, this phase also adds on-disk checkpointing before merge.

**Phase 4 — `summary` field + overflow compaction wiring.** Populate `ConversationMeta.summary` during compaction. Thread it into `search_sessions` as the 3× boost field. Nextest coverage: session with a summary ranks above an otherwise-equivalent session without one for the same query.

**Phase 5 — Documentation.** Update `CLAUDE.md` / `CONTRIBUTING.md` on the runtime-vs-client boundary. Update hook examples that referenced topics. Move 0171 into `superseded.md`.

## Open questions

- **`EntryKind::Topic` fate.** Keep as a purely presentational label for long-form aggregated entries, or delete entirely and treat "wiki" entries as ordinary `project` entries? The label earns its keep only if a UI or search-ranking consumer branches on it. Current lean: delete in a follow-up once Phase 1–5 are stable and we can confirm no consumer actually reads the tag.
- **On-disk index checkpointing.** Governed by the Phase 3 bench. If cold-start stays within budget, defer; if not, land it inline. Decision deferred to measurement, not debate.
- **Session BM25 field-weight calibration.** Adopt community defaults as-is. A labeled test set of ≥50 queries with known-relevant sessions triggers a re-tuning pass if agent recall on that set falls below 80% top-3 hit rate. Until that set exists, the weights are frozen.
- **Discretionary `compact(session_id)` helper.** Ship only when a client demands it. Not in this RFC.
