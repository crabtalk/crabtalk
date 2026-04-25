## search_sessions

Search past conversations by keyword to recall related work or
prior decisions. Returns ranked **excerpts** — the matched message
plus surrounding context — not full session histories.

When to use:

- The user asks about prior conversations ("what did we decide",
  "we talked about this last week").
- You suspect related context exists from earlier sessions and want
  to confirm before re-deriving from scratch.
- You need to find an example, command, or fix from past work.

Hit shape:

- Each hit names the session handle, the agent and sender, the
  matched message index, and a window of surrounding messages.
- Snippets are bounded — long messages are truncated with `…`.
- Best-hit-per-session: a single session won't dominate results.

Tips:

- Start with the user's own terms; keep queries short (2–6 words).
- Filter by `agent` or `sender` when you know the conversation
  partner.
- Increase `context_before`/`context_after` if a hit looks promising
  but lacks enough surrounding context. Don't request huge windows —
  the cap is enforced server-side.

Limits:

- Returns up to 20 hits, each with up to 16 messages of window.
- Snippets are truncated at 1024 bytes.
- Auto-injected entries (recall blocks, env blocks) are not
  indexed — you won't see them in results.
- Tool-result content and tool-call arguments are excluded from
  the search index — they often contain credentials or other
  sensitive output. Tool **names** are still searchable, so
  "find sessions where I ran `shell`" works. Window context
  still shows tool output for matched conversations, which is
  the same boundary as resuming the session.
