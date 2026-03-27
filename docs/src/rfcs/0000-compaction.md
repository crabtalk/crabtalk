# 0000 - Compaction

- Feature Name: Auto-Compaction
- Start Date: 2025-12-01
- Discussion: foundational design
- Crates: core

## Summary

Automatic context management for conversations that outgrow the LLM's context
window. When history exceeds a token threshold, the agent uses the LLM itself
to summarize the conversation into a compact briefing that replaces the full
history. The conversation continues with no interruption.

## Motivation

LLM context windows are finite. A conversation that runs long enough — multi-step
tool use, research sessions, debugging loops — will exceed the model's limit.
When that happens, the request fails. The user loses their session.

Every LLM application has to solve this problem. The common approaches are:

- **Truncation** — drop old messages. Cheap but lossy. The agent forgets
  decisions, context, and user preferences from earlier in the conversation.
- **Sliding window** — keep the last N messages. Same problem: the agent loses
  the beginning of the conversation.
- **Retrieval** — embed messages and retrieve relevant ones. Heavyweight:
  requires a vector store, an embedding model, and a retrieval strategy.

Crabtalk's approach: **use the LLM to summarize itself.** The same model that's
having the conversation produces a dense summary of everything important. The
summary replaces the history. The conversation continues as if nothing happened.

## Design

### Trigger

After each agent step (LLM response + tool results), the runtime estimates the
token count of the current history. If it exceeds `compact_threshold` (default
100,000 tokens), compaction fires automatically.

Token estimation is a heuristic: ~4 characters per token, counting message
content, reasoning content, and tool call arguments. It's deliberately rough —
the threshold is a safety margin, not a precise limit.

### Compaction

The agent sends the full history to the LLM with a compaction system prompt
that instructs it to:

**Preserve:**
- Agent identity (name, personality, relationship notes)
- User profile (name, preferences, context)
- Key decisions and their rationale
- Active tasks and their status
- Important facts, constraints, and preferences
- Tool results still relevant to ongoing work

**Omit:**
- Greetings, filler, acknowledgements
- Superseded plans or abandoned approaches
- Tool calls whose results have been incorporated

The compaction prompt also includes the agent's system prompt, so the LLM
preserves identity and profile information from `<self>`, `<identity>`, and
`<profile>` blocks.

The output is dense prose, not bullet points — it becomes the new conversation
context and must be self-contained.

### Replacement

After compaction:

1. The summary is yielded as an `AgentEvent::Compact { summary }`.
2. The session history is replaced with a single user message containing the
   summary.
3. A `[context compacted]` text delta is yielded so the user sees it happened.
4. The agent loop continues — the next step sees the compact summary as its
   entire history.

On disk, a `{"compact":"..."}` marker is appended to the session JSONL. On
reload, `load_context` reads from the last compact marker forward. History
before the marker is archived in place — still in the file, never deleted.

### Interaction with other systems

- **Memory auto-recall** — runs fresh every turn via `on_before_run`. Compaction
  doesn't affect recall — memories are separate from conversation history.
- **Client-initiated compact** (RFC 0078) — the same `Agent::compact()` method,
  but triggered by the client for @-mention handoff rather than by the token
  threshold.
- **Session persistence** — compact markers are append-only in the JSONL. The
  full history survives on disk even after in-memory replacement.

### Configuration

Per-agent configurable. `None` disables auto-compaction. The default of 100,000
tokens leaves headroom below most model context limits (128K–200K) for the
system prompt, tool schemas, and injected context.

## Alternatives

**Truncation / sliding window.** Cheap but the agent loses context. In a
multi-step debugging session, forgetting the first half of the investigation
means repeating work. Compaction preserves the substance while discarding the
noise.

**RAG over message history.** Retrieve relevant messages via embeddings. More
precise than compaction but requires infrastructure (vector store, embedding
model) and adds latency to every turn. Compaction is zero-infrastructure — it
uses the model already in the conversation.

**No automatic compaction.** Let the user manage context manually. Rejected
because context overflow is invisible until the request fails. The user
shouldn't need to monitor token counts.

## Unresolved Questions

- Should the compaction prompt be customizable per agent?
- Should the threshold adapt based on the model's actual context limit rather
  than a fixed number?
