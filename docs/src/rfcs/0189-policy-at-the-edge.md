# 0189 - Policy at the Edge

- Feature Name: Policy at the Edge
- Start Date: 2026-04-28
- Discussion: [#188](https://github.com/crabtalk/crabtalk/issues/188), [#189](https://github.com/crabtalk/crabtalk/pull/189)
- Crates: core, runtime, crabtalk, sdk
- Supersedes: [0000 (Compaction)](0000-compaction.md)
- Updates: [0075 (Hook)](0075-hook.md), [0150 (Memory Store)](0150-memory-store.md), [0185 (Session Search)](0185-session-search.md)

## Summary

Mechanism belongs in the daemon; policy belongs at the edge. The daemon stops making decisions on the user's behalf — it no longer auto-compacts on a token-count heuristic, no longer spawns title-generation calls in the background, no longer BM25-searches memory and injects synthetic `<recall>` user turns. Each of these is now an explicit RPC the client calls when (and if) it wants the behavior. A new `AgentEvent::ContextUsage { usage }` carries real per-step token counts so clients can pick their own pressure threshold. The `Hook::on_before_run` lifecycle method is removed.

## Motivation

Three independent features had drifted toward the same anti-pattern: the daemon making policy decisions using its own heuristics, then mutating conversation state on the user's behalf without being asked. RFC 0000 codified auto-compaction at a `chars/4`-derived threshold. RFC 0038 (then 0150) codified auto-recall as a per-turn before-run injection. The runtime grew a quiet `spawn_title_generation` call inside `finalize_run`. Each was useful in isolation. Together they shaped a daemon that thought it knew best.

The cost of that posture:

- **Bad heuristics.** Token estimation as `chars/4` is wrong for code, JSON tool outputs, and non-English prose. The threshold either trips early (destroying live context with an unwanted summary) or trips late (the request fails anyway). The daemon doesn't have the inputs — model identity, real token counts, user intent — to pick a threshold. Clients do.
- **Synthetic events.** Auto-compaction yielded `AgentEvent::Compact` followed by hand-forged `TextStart`/`TextDelta`/`TextEnd` events containing the literal string `[context compacted]`. Auto-recall injected `<recall>...</recall>` user turns flagged `auto_injected: true`. Both lied to the event stream — the model didn't say those things, the daemon did. Downstream consumers had to filter them out.
- **Wasted tokens, opaque costs.** Auto-titling spent an LLM call after every conversation that crossed two history entries, behind the user's back. Auto-recall paid retrieval cost on every turn whether or not the model would have asked.
- **Race with the explicit API.** All three behaviors had explicit-API counterparts (`compact_conversation`, the `recall` tool, a clearly-named title RPC if the client wanted one). The daemon was racing the client to call its own API.

RFC 0185 already drew the right line for sessions: "the runtime's job is to provide mechanical primitives. UX decisions belong one layer up in the client." This RFC carries that all the way through.

## Design

### Principle

Mechanism in the daemon, policy at the edge. Concretely:

- **Mechanism** is what only the daemon can do: own conversation state, own storage, own the LLM connection, own MCP child processes, run summarization, write archives. These are inherently centralized.
- **Policy** is everything else: when to compact, when to title, what to prepend to a user message, what counts as context pressure. These need information the daemon doesn't have (which model, which UI, which user, which tradeoff matters today). Policy lives in the client — TUI, telegram, web app, headless automation — and is composed from primitives the daemon exposes.

Where this leaves heuristics: the daemon doesn't run them. If the daemon would need to estimate something to decide, the answer is "don't decide — surface the data and let the client decide."

### What was removed

**Auto-compaction.** The block in `Agent::run` that called `self.compact(history)` when `estimate_tokens(history) > threshold` is gone. The synthetic `Compact`/`TextStart`/`TextDelta(\"[context compacted]\")`/`TextEnd` events are gone. `AgentConfig::compact_threshold` is gone (silently dropped from existing TOML via serde default). `HistoryEntry::estimate_tokens` and the `chars/4` heuristic are gone.

**Auto-titling.** `Runtime::spawn_title_generation` and its `finalize_run` call site are gone. The `title` field on `Conversation` and `ConversationMeta` stays — existing data is still valid, the daemon just doesn't generate fresh titles on its own.

**Auto-recall.** `Memory::before_run` (the BM25-search-and-inject helper) is gone. `MemoryHook::on_before_run` is gone. The `recall` tool is unchanged — model-driven recall continues to work.

**`Hook::on_before_run`.** The trait method is removed. `OsHook` previously used it to inject `<environment>working_directory: ...</environment>` per turn — that goes too. Bash dispatch still resolves the effective cwd at tool-call time, so commands run in the right directory; the model just doesn't get a synthetic turn telling it where it is. Clients that want the model to see the cwd put it in their own user message (they supplied it via `req.cwd` in the first place). The peer-agents `<agents>` block that `DaemonHook::on_before_run` injected for delegation moves to `DaemonHook::on_build_agent` so it lands in the system prompt at agent-build time — registry mutations are visible after the next agent rebuild.

### What was added

**`AgentEvent::ContextUsage { usage: Usage }`.** Emitted once per LLM call when the provider reports non-zero usage. Carries real `prompt_tokens`, `completion_tokens`, `total_tokens`, plus optional cache-hit/miss and reasoning counts. The corresponding wire event is `ContextUsageEvent { usage: TokenUsage }`. Clients track these and decide for themselves when to call `compact_conversation`.

**Real `compact_conversation`.** The runtime method previously returned the summary string and silently dropped the persistence work. It now does all four steps in order: summarize → write archive entry → write session compact marker → replace history with a single user message carrying the summary. Atomic from the client's perspective.

### Reference: explicit replacements

Each removed behavior maps to an existing or planned API:

| Removed | Explicit replacement |
|---------|----------------------|
| Auto-compaction | `compact_conversation(agent, sender)` RPC, gated on client-tracked `ContextUsage` events |
| Auto-titling | A future `generate_title(conversation_id)` RPC; until then, clients can run their own summarization or leave titles blank |
| Auto-recall | The `recall` tool (model-driven); or a client-side `recall + send` composition before the user's message |

The opt-in client-side helpers for each of these are tracked in [#188](https://github.com/crabtalk/crabtalk/issues/188) as SDK sugars — a few dozen lines on top of the daemon client.

## Migration

- New conversations have empty `title` until a client asks for one. Existing titles on disk are unaffected.
- The `recall` tool still works. Clients that previously relied on silent `<recall>` injection need to either let the model call `recall` itself (the intended path) or compose `recall + send` client-side.
- No auto-compact. Clients should subscribe to `ContextUsage` events and call `compact_conversation` when their threshold trips. The model returns an explicit error if context is exceeded — the daemon no longer guesses.
- `compact_threshold` in agent TOML is silently dropped via serde default. No errors, just ignored.

## Alternatives considered

**Keep auto-compact as a safety net.** RFC 0185 took this position: "automatic compaction on overflow as a safety net" because clients can't see overflow coming. Rejected here because the daemon can't reliably detect overflow either — `chars/4` is the wrong tool, and the model itself returns a clear error when context is exceeded. A bad safety net is worse than none, because clients build trust in it and stop watching.

**Threshold-gated `ContextPressure` event.** Emit only when over some threshold. Rejected because it recreates the policy problem in a smaller form — the daemon still picks a number, and is still wrong for whichever model and use case it didn't anticipate. Always-emit `ContextUsage` lets clients pick.

**Move policy to per-agent config knobs.** "Auto-compact off by default; opt in via `compact_threshold`." Rejected because the per-agent config is set by the client at create-time anyway — moving the decision a step earlier doesn't change who decides, just makes the decision harder to update. A per-call decision (the client picks each turn) is more honest.

## Out of scope

Two daemon-side per-turn injections in `prepare_history` survive this RFC: the `<instructions>` block from Crab.md discovery and the guest-agent-framing prose ("Messages wrapped in `<from agent=\"...\">`..."). Same anti-pattern, deferred to a separate cleanup so this RFC stays focused.

Wire-protocol changes are limited to the new `ContextUsageEvent` and reservation of `AgentInfo.compact_threshold` (field 10). No breaking renumbering, no new RPCs.
