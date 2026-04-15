# Multi-agent

Multi-agent conversations let a second agent speak into an existing conversation as a guest. A guest turn is a first-class message from the guest agent; it is not a tool call, a delegation, or a paraphrase.

## Guest turns

A guest turn runs a named guest agent against the primary conversation's history and appends the guest's response to that history. The primary agent of the conversation is unchanged.

A guest turn is requested by setting `StreamMsg.guest` to the name of the guest agent. The conversation is still addressed by the primary's `(agent, sender)` pair; `guest` selects who speaks on this turn, not whose conversation it is.

## Flow

When `StreamMsg { agent: A, sender: S, guest: G, content: C }` is dispatched:

1. The conversation `(A, S)` is resolved, creating it if necessary.
2. The user content `C` is appended to the history.
3. The daemon runs agent `G` against the history using `G`'s system prompt and instructions.
4. The response is appended to the history, tagged with `agent: G`.

The primary agent is not invoked on a guest turn. A subsequent `StreamMsg` without `guest` resumes normal operation with the primary agent against the updated history.

## Tools on guest turns

A guest turn is text-only. The guest agent's tool schemas are not attached to the request, and any tool call emitted by the guest is rejected.

Tool-using work belongs to the primary agent. A guest is a voice in the conversation, not a worker.

## Attribution

Each message in the history carries an `agent` field.

- `agent` empty — the message originates from the conversation's primary agent.
- `agent` non-empty — the message originates from a guest. The value is the guest agent's name.

Attribution survives compaction: archive entries preserve the `agent` field of each archived message.

## Framing

When building a request, the runtime auto-injects framing messages that are not persisted between runs. Two framings exist:

- **Guest framing.** Injected when a guest is running. It tells the guest that it is joining a conversation and explains the `<from agent="...">` tag convention.
- **Primary framing.** Injected when the primary is running and the history contains at least one message with a non-empty `agent`. It tells the primary that some messages are from guest agents and it should continue responding as itself.

Framing messages are marked auto-injected. They are stripped from the history at the start of each run and re-injected for that run only. The history on disk never contains framing messages.

## Tagging

Assistant messages with a non-empty `agent` field are prefixed with `<from agent="{name}">` when they appear in an LLM request. The prefix makes the speaker visible to whichever agent is currently reading the history.

A message without an `agent` field carries no prefix.

## Cancellation

`KillMsg` addresses the conversation by `(agent, sender)`. It cancels whichever run is in flight, whether that run is the primary or a guest. A cancelled guest turn leaves the user's content appended to the history; the guest's partial response is discarded.
