# 0171 - Topic Switching

- Feature Name: Topic Switching
- Start Date: 2026-04-19
- Discussion: [#171](https://github.com/crabtalk/crabtalk/issues/171)
- Crates: memory, runtime, crabtalk
- Updates: [0135 (Agent-First)](0135-agent-first.md), [0150 (Memory Store)](0150-memory-store.md)

## Summary

Topic switching is a first-class conversation boundary: the agent can partition its work with a person into parallel threads, each with its own history and compaction archive. Agent-First (0135) established `(agent, sender)` as the conversation key; this RFC layers `topic` on top so one `(agent, sender)` pair maps to N conversations keyed by title, plus an active-topic pointer. A new tool pair — `search_topics` and `switch_topic` — lets the agent drive it. Untopicked chats are tmp: in-memory only, no storage I/O, dropped at the end of the run.

## Motivation

Compaction is the only conversation boundary we had before this work, and it's the wrong tool for the job. Compaction is context-window management — it fires when the transcript gets full, not when the topic changes. Work on auth and work on the deploy pipeline end up in the same conversation history, competing for the same context window, confusing each other.

Compact and topic-switch are different mechanisms:

- **Compact** is intra-conversation. Summarize the current thread, archive the summary, keep going. Same conversation id, same active state.
- **Topic switch** is inter-conversation. Pause this thread (full context preserved), pick up another. No summarization, no archive. Each topic is a conversation with its own compaction history.

## Design

### Tools

Two new agent tools:

- `search_topics(query)` — BM25 search over existing topics. Returns ranked `(title, description)` pairs. Not a `list_topics` dump — the agent searches, not browses, same as `recall`.
- `switch_topic(title, description?)` — exact title match resumes that conversation and makes it active; no match creates a new conversation with this title. `description` is required on create, ignored on resume.

Free-form titles, agent-chosen. The title *is* the key.

### Agent-written descriptions

When the agent creates a new topic, it supplies a short description — one to three sentences on what the topic is about. That description becomes the `content` of the memory entry and is what BM25 actually indexes. Title alone is too short for useful recall; auto-summarization from conversation content would need an LLM round-trip per switch and drift as the conversation evolves. The description is immutable after creation — if the focus shifts, the agent switches to a new topic rather than rewriting the label.

### Memory integration

Topics piggyback on the memory crate. A new `EntryKind::Topic` variant joins `Note` and `Archive`. Each topic materializes as a memory entry:

- `name` = the topic title
- `content` = the agent-written description
- `kind` = `EntryKind::Topic`

`search_topics` is implemented as `Memory::search_kind(query, limit, EntryKind::Topic)` — one BM25 index, one ranking story. When the scope-weighting work in #170 lands, recency decay means recently-touched topics rank above stale ones for free.

### Runtime routing

The daemon tracks, per `(agent, sender)`:

- `TopicRouter { by_title: HashMap<String, ConversationId>, active: Option<String>, tmp: Option<ConversationId> }`

`get_or_create_conversation(agent, sender)` routes in order:

1. If `router.active = Some(title)` and the title is in `by_title`, return that conversation.
2. Otherwise, return or create the tmp conversation.

Protocol surface is unchanged: clients still address `(agent, sender)` and `StreamMsg` gains no topic field. The daemon routes to the active topic's conversation; the agent owns topic decisions; the user just talks to the agent.

### Tmp chats are not persisted

A conversation with no topic has no session handle. `ensure_handle` refuses to create one for tmp chats, and `persist_messages` no-ops without a handle. This is a deliberate, backward-incompatible break from Agent-First, which always auto-resumed the latest session on first message. Under the new model:

- A user who never switches to a topic never persists anything. Their work is in-memory for the life of the run, then gone.
- A user who wants continuity calls `switch_topic`. That session is persisted; prior topic sessions for the same `(agent, sender, title)` are resumed via a `list_sessions()` scan on cold switch.

This trades reflexive persistence for agent-controlled persistence. The agent now has to decide what's worth keeping, which is the right place for that decision.

### Cold-path concurrency

`switch_active_topic` on an unknown title would otherwise race: two callers both miss the fast path, both do storage I/O, both `create_session`, second clobbers first. Fix: reserve the conversation id under the router's write lock *before* any I/O. Any concurrent caller sees the reservation and resumes to the same conversation instead of creating a duplicate. If the I/O subsequently fails, the reservation is rolled back.

### Compaction per topic

Each topic compacts independently. The existing `AgentConfig::compact_threshold` fires on the active topic's conversation and writes an `EntryKind::Archive` entry scoped to that topic — exactly as today, just per-topic instead of per-`(agent, sender)`.

Archives are named `{topic-slug}-{n}` where `n` is the next free sequence number for this topic. Older archives stay searchable via `recall` instead of being overwritten, so the agent can surface older phases of a long-running topic when they're relevant. Scan and insert happen under one memory write lock so two concurrent compactions can't pick the same sequence.

Resume auto-prepends the right archive: `load_session` returns the most recent compact marker, `resumed_history` looks that archive up in memory and injects its content as the replayed prefix. Topic switching just changes which conversation is active; the resume mechanism is unchanged.

`switch_topic` does **not** implicitly compact. If a resumed topic is close to threshold, normal auto-compaction fires on the next turn — late enough to stay cheap, early enough to matter.

Downstream ranking work lives in [#170](https://github.com/crabtalk/crabtalk/issues/170): topic becomes a new scope dimension (same-topic boost, related-topic BM25 similarity) when that RFC lands.

## Open questions

- **Deletion.** Forgetting a topic entry via `forget` orphans the underlying conversation (runtime keeps the router until process exit). Whether a topic should be truly deletable — and what happens to its archive history — is deferred.
- **Cross-restart active-topic memory.** The active-topic pointer lives only in the running process. On restart, every `(agent, sender)` starts in tmp until the agent re-enters a topic. Rebuilding "last active topic per pair" from storage is doable but not part of this RFC.
