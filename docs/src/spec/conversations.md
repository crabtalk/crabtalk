# Conversations

A conversation is the unit of agent interaction. It holds the message history an agent uses as working context, together with the state associated with that history.

## Identity

A conversation is identified by the pair `(agent, sender)`.

- `agent` is the name of an agent configured in the daemon.
- `sender` is a client-provided string identifying the counterparty. Clients choose their own convention, such as `"user"`, `"tg:12345"`, or `"delegate:42"`.

The pair is the conversation's only externally addressable name. The wire protocol carries no conversation identifier.

## Lifetime

A conversation is created on first reference to a pair `(agent, sender)` that does not yet exist, and persists across daemon restarts. Persistence is delegated to the configured `Storage` backend.

At most one conversation exists for any given `(agent, sender)` pair.

## Addressing

Protocol messages that operate on a conversation carry `agent` and `sender` fields. The pair resolves to the conversation on which the operation acts.

| Message      | Effect                                                        |
|--------------|---------------------------------------------------------------|
| `StreamMsg`  | Append user content, run the agent, stream the response.      |
| `KillMsg`    | Cancel the in-flight run, if any.                             |
| `CompactMsg` | Compact the current history into an archive (see Memory).     |
| `ReplyToAsk` | Supply content for a pending `ask_user` call.                 |

`StreamMsg.sender` is optional. When omitted, the daemon resolves a default sender determined by the transport.

## State

A conversation holds:

- **History** — an ordered sequence of history entries.
- **Title** — a short human-readable label assigned by the `set_title` tool.
- **Working directory** — the filesystem path used by OS-level tools during a run.
- **Archives** — compacted prefixes of the history (see Memory).

History ordering is total. New entries are appended; no entry is reordered or removed except through compaction.

## Working directory

Each conversation has a default working directory. `StreamMsg.cwd`, when set, overrides the default for the duration of the resulting run. The override does not modify the conversation's default.

## Message attribution

Each assistant message in the history carries an `agent` field.

- An empty `agent` field denotes a message produced by the conversation's primary agent, the one named by the conversation's identity.
- A non-empty `agent` field denotes a guest turn (see Multi-agent).

Messages produced by the daemon for protocol framing are marked as auto-injected and stripped from the history before each run.
