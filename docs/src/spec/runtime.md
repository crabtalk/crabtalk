# Runtime

The runtime owns the architecture of *lifetime*: it holds live conversations, runs agent steps, dispatches tool calls, and applies compaction. It does not open sockets, accept connections, or schedule time. Capabilities that require I/O are provided to the runtime by its environment.

It is also the thing that eats harnesses. Features do not belong here — a feature that ends up as a runtime field is a feature that leaked into the lifecycle engine.

## Composition

A runtime is parameterized by a `Config` that names three associated types:

| Type       | Responsibility                                      |
|------------|-----------------------------------------------------|
| `Storage`  | The store: one `KVStorage` impl, and every interface built on it. |
| `Provider` | LLM request and streaming.                          |
| `Env`      | Node-specific capabilities and tool dispatch.       |

A binary supplies one `Config`. The shipped `Config` wires `crabdb`, a configured provider, and a node environment that owns harnesses and event broadcasting. Tests supply a `Config` with an in-memory store, a stub provider, and `()` as the environment.

The runtime holds interfaces, never data. There is no agent registry and no memory handle: an agent is read from the store for the run that needs it, built, and dropped. Whether any of it is cached is the store's decision, which is what makes a different deployment a different implementation rather than a rewrite of the runtime. The exception is a live session, which holds a steering channel — a channel cannot be persisted, so it is genuinely per-process state.

## Responsibilities

The runtime handles:

- Loading and saving conversations through `Storage`.
- Building an agent request from the current history, instructions, and tool schemas.
- Streaming responses from `Provider` and applying them to the conversation.
- Dispatching tool calls through `Env`.
- Emitting `AgentEvent` values for each step, tool call, and compaction.
- Producing compaction summaries and appending archive markers.

## Conversations

A conversation is the unit of agent interaction: the message history an agent
uses as working context, plus the state that travels with it. The runtime holds
live conversations; the protocol addresses them by `(agent, sender)`.

### Lifetime

A conversation is created on first reference to a pair `(agent, sender)` that does not yet exist, and persists across daemon restarts. Persistence goes through the `Sessions` interface; the store behind it is the binary's choice.

At most one conversation exists for any given `(agent, sender)` pair.

### State

A conversation holds:

- **History** — an ordered sequence of history entries.
- **Title** — a short human-readable label.
- **Archive** — a pointer to the compacted prefix of the history, if the conversation has been compacted (see Memory).

History ordering is total. New entries are appended; no entry is reordered or removed except through compaction.

### Message attribution

Each assistant message in the history carries an `agent` field.

- An empty `agent` field denotes a message produced by the conversation's primary agent, the one named by the conversation's identity.
- A non-empty `agent` field denotes a guest turn (see Multi-agent).

Messages produced by the daemon for protocol framing are marked as auto-injected and stripped from the history before each run.

### Guest turns


A guest turn runs a named guest agent against the primary conversation's history and appends the guest's response to that history. The primary agent of the conversation is unchanged.

A guest turn is requested by setting `StreamMsg.guest` to the name of the guest agent. The conversation is still addressed by the primary's `(agent, sender)` pair; `guest` selects who speaks on this turn, not whose conversation it is.

#### Flow

When `StreamMsg { agent: A, sender: S, guest: G, content: C }` is dispatched:

1. The conversation `(A, S)` is resolved, creating it if necessary.
2. The user content `C` is appended to the history.
3. The daemon runs agent `G` against the history using `G`'s own description, which is its system message.
4. The response is appended to the history, tagged with `agent: G`.

The primary agent is not invoked on a guest turn. A subsequent `StreamMsg` without `guest` resumes normal operation with the primary agent against the updated history.

#### Tools on guest turns

A guest turn is text-only. The guest agent's tool schemas are not attached to the request, and any tool call emitted by the guest is rejected.

Tool-using work belongs to the primary agent. A guest is a voice in the conversation, not a worker.

#### Framing

When building a request, the runtime auto-injects framing messages that are not persisted between runs. Two framings exist:

- **Guest framing.** Injected when a guest is running. It tells the guest that it is joining a conversation and explains the `<from agent="...">` tag convention.
- **Primary framing.** Injected when the primary is running and the history contains at least one message with a non-empty `agent`. It tells the primary that some messages are from guest agents and it should continue responding as itself.

Framing messages are marked auto-injected. They are stripped from the history at the start of each run and re-injected for that run only. The history on disk never contains framing messages.

#### Tagging

Assistant messages with a non-empty `agent` field are prefixed with `<from agent="{name}">` when they appear in an LLM request. The prefix makes the speaker visible to whichever agent is currently reading the history.

A message without an `agent` field carries no prefix.

## Boundary

The runtime does not:

- Bind listeners or accept transport connections.
- Spawn tasks for message routing or scheduling.
- Interpret protocol messages.
- Read the system clock for scheduling purposes.
- Manage process state such as PID files or signals.

These belong to the server that hosts the runtime.

## Env

`Env` is the runtime's only outward-facing capability surface. It provides:

- `hook()` — the composite `Harness` that exposes tool schemas, dispatches tool calls, and participates in lifecycle events.
- `on_agent_event(agent, conversation_id, event)` — hook point for side effects, such as event broadcasting or persistence of step traces.
- `subscribe_events()` — optional subscription to a cross-conversation event stream, for servers that expose agent events to external clients.
Methods that the runtime does not need in a given context have default implementations. An `Env` implementation may leave event broadcasting at its default.

Instruction discovery and working-directory resolution are not here. The daemon does not read the user's filesystem — a client renders local instructions into the message it sends, and a harness reaches files through the root its declaration grants.

## Harness

A harness is a way to serve a tool call the daemon does not implement itself.

`Harness` is public API at the runtime layer, and that is the point: an embedder
using crabtalk as a library implements it, registers it through the composite,
and gets tools in their own process without running a daemon. That is a
consumer the protocol cannot serve — a client on a socket and a crate in your
binary want different things.

`Harness` is the single point through which the runtime reaches node-specific tools. A harness:

- Advertises tool schemas for the LLM request.
- Dispatches tool calls by name, returning a future that yields the tool's result.
- Participates in step lifecycle, observing starts, completions, and errors.

It is composite: the daemon's owns sub-harnesses, and the runtime sees a single `Harness`. Order is fixed by the composite.

`usage` is the one declaration worth calling out — what these tools are for, when to reach for them, and how they go together. It is the question no single tool's `description` answers, because it is about choosing between them.

### Why two ship

`dispatch` is the only method on the trait that genuinely requires it —
`schema` and `usage` are declarations, and the rest is lifecycle bookkeeping.
So the question "how many implementations are there" is really "how many ways
can a tool call be served that the daemon does not implement", and there are
two: **inside a sandbox** or **over the network** (MCP). A third would need a
third execution substrate.

An embedder adding their own is not a violation of that count. It is the seam
working.

## Tool dispatch

A tool call from the agent carries the tool name, arguments, the originating agent and sender, and the conversation id. The runtime invokes `Env::hook().dispatch(name, call)`. If no sub-harness claims the name, the dispatch yields an error result; the agent receives the error as the tool's output.

Dispatch is asynchronous. The runtime awaits the tool future at the next step boundary and applies the result to the conversation before the following step.
