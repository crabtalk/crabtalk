# 0001 - Compact Session

- Feature Name: Compact Session Interface
- Start Date: 2026-03-25
- Discussion: [#78](https://github.com/crabtalk/crabtalk/issues/78)

## Summary

Expose session compaction as a protocol operation so clients can request a
concise context summary on demand, enabling cross-agent context handoff with
custom @-mention logic.

## Motivation

When a user @-mentions a different agent mid-conversation, the client needs to
hand off context. The naive approaches don't work:

- **Raw history** includes irrelevant tool results, thinking tokens, and the
  previous agent's system prompt — expensive and noisy.
- **No context** means the target agent flies blind.

Compact produces a focused briefing: the LLM summarizes the conversation into
essential context. The target agent gets its own system prompt (warm in token
cache) plus the compact summary plus the user's query — high quality context,
minimal tokens.

The key insight: **this belongs in the protocol, not the client.** The daemon
already has the session history and the LLM connection. The client just needs
to say "compact session N" and get a summary back. But the **mention logic
itself stays in the client** — the daemon doesn't know about @-mentions, UI
conventions, or which agent to route to. The client decides when and why to
compact; the daemon does the summarization.

## Design

A `Compact` message is added to the protobuf protocol:

- **Request:** `CompactRequest { session: u64 }` — client asks the daemon to
  compact a specific session.
- **Response:** `CompactResponse { summary: string }` — the daemon returns the
  summarized context.

The `Server` trait gains a `compact_session` method. The daemon implementation
delegates to `Agent::compact()`, which sends the session history to the LLM
with a compaction prompt that preserves identity and profile information.

### What the daemon does

- Accepts the compact request via the protocol.
- Loads the session history.
- Calls the agent's compact method (LLM summarization).
- Returns the summary string.

### What the client does

- Detects @-mentions (its own UI logic).
- Requests compact of the current session.
- Creates or selects the target agent's session.
- Sends the compact summary + user query to the target agent.

### Context selection alternatives

If compact is too slow for the use case:

- **BM25** — already in the codebase for memory recall. Keyword-match messages
  against the query.
- **Last N messages** — simplest. Often sufficient for short conversations.

These are client-side decisions. The compact interface doesn't preclude them.

## Alternatives

**Client-side compaction.** The client could do its own summarization, but it
would need LLM access and session history — duplicating what the daemon already
has.

**Automatic compaction on mention.** The daemon could detect @-mentions and
compact automatically. Rejected because mention syntax is a client concern —
different clients have different conventions.

## Unresolved Questions

- Should compact accept parameters (max tokens, focus query) to guide
  summarization?
- Should the daemon cache compact results for repeated handoffs within the same
  conversation?
