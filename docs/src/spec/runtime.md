# Runtime

The runtime is the engine that drives agents. It owns conversations in memory, runs agent steps, dispatches tool calls, and applies compaction. It does not open sockets, accept connections, or schedule time. Capabilities that require I/O are provided to the runtime by its environment.

## Composition

A runtime is parameterized by a `Config` that names three associated types:

| Type       | Responsibility                                      |
|------------|-----------------------------------------------------|
| `Storage`  | Persistence of conversations, skills, and memory.   |
| `Provider` | LLM request and streaming.                          |
| `Env`      | Node-specific capabilities and tool dispatch.       |

A binary supplies one `Config`. The daemon's `Config` wires filesystem storage, a configured provider, and a node environment that owns hooks and event broadcasting. Tests supply a `Config` with in-memory storage, a stub provider, and `()` as the environment.

## Responsibilities

The runtime handles:

- Loading and saving conversations through `Storage`.
- Building an agent request from the current history, instructions, and tool schemas.
- Streaming responses from `Provider` and applying them to the conversation.
- Dispatching tool calls through `Env`.
- Emitting `AgentEvent` values for each step, tool call, and compaction.
- Producing compaction summaries and appending archive markers.

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

- `hook()` — the composite `Hook` that exposes tool schemas, dispatches tool calls, and participates in lifecycle events.
- `on_agent_event(agent, conversation_id, event)` — hook point for side effects, such as event broadcasting or persistence of step traces.
- `subscribe_events()` — optional subscription to a cross-conversation event stream, for servers that expose agent events to external clients.
- `discover_instructions(cwd)` — collect instruction files applicable to a working directory.
- `effective_cwd(conversation_id)` — resolve the working directory for a run, honoring any per-conversation override.

Methods that the runtime does not need in a given context have default implementations. An `Env` implementation may leave event broadcasting, instruction discovery, or CWD management at their defaults.

## Hook

`Hook` is the single point through which the runtime reaches node-specific tools. A hook:

- Advertises tool schemas for the LLM request.
- Dispatches tool calls by name, returning a future that yields the tool's result.
- Participates in step lifecycle, observing starts, completions, and errors.

A hook is composite: the daemon's hook owns sub-hooks (OS tools, `ask_user`, delegation, event subscription, memory). Order of sub-hooks is fixed by the composite; the runtime sees a single `Hook`.

## Tool dispatch

A tool call from the agent carries the tool name, arguments, the originating agent and sender, and the conversation id. The runtime invokes `Env::hook().dispatch(name, call)`. If no sub-hook claims the name, the dispatch yields an error result; the agent receives the error as the tool's output.

Dispatch is asynchronous. The runtime awaits the tool future at the next step boundary and applies the result to the conversation before the following step.
