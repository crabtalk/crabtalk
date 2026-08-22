# Daemon

The daemon is the long-lived process that hosts the runtime, owns transports, and persists state. Clients are transient; the daemon is not. A single daemon process serves all configured agents, all active conversations, and all connected clients.

## Responsibilities

The daemon owns:

- **Transports** — UDS and TCP listeners. Listening endpoints belong to the daemon, not to individual clients or agents.
- **Runtime** — a single shared runtime instance behind `RwLock`. Agents share the runtime; the runtime is never cloned per conversation.
- **Harnesses** — the composite `Harness`. Two ship today: the one surfacing the tools of each agent's declared harnesses, and MCP. `Harness` is public API at the runtime layer, so an embedder registers their own.
- **Event bus** — subscription table and fire callback. File-backed by `events/subscriptions.toml` under the config directory.
- **MCP handler** — connections to external MCP servers and routing to the tools they advertise.
- **Harness images** — compiled RV64 ELFs, keyed by a content digest of the ELF, its grants, and the scope its capabilities close over.
- **Configuration** — current `DaemonConfig`, reloaded in place on explicit reload.

The daemon does not interpret tool semantics. Tool dispatch is the runtime's responsibility, routed through the composite.

The daemon owns no tools of its own. `bash`, `read` and `edit` are a harness an agent declares. What the daemon supplies is the socket, the runtime, and the state — not a set of capabilities it decided every agent should have.

## Process model

The daemon runs as a single OS process. All work happens on a single Tokio runtime. There is one listener task per configured transport, one reply task per connected client, and one task per in-flight dispatch. Shutdown is initiated by a broadcast channel; every long-lived task subscribes and exits when the channel fires.

A daemon process owns at most one configuration directory and at most one set of transport endpoints.

## Config directory

The daemon is rooted at a configuration directory supplied at startup. The directory holds:

| Path                           | Contents                                            |
|--------------------------------|-----------------------------------------------------|
| `config.toml`                  | Node configuration, hand-written and read on reload. |
| `store.crmem`                  | The store: agents, sessions, memory, skills, search. |
| `events/subscriptions.toml`    | Event subscription recovery file.                   |

All paths are resolved relative to the configuration directory. The daemon writes nothing outside this directory.

## Lifecycle

**Startup.** The daemon reads `config.toml`, constructs the provider, assembles harnesses, opens storage, builds the shared runtime, loads event subscriptions from disk, binds transports, and begins accepting client messages.

**Runtime.** The daemon serves the `Server` trait. Each client message is dispatched into a spawned task that produces a stream of server messages.

**Reload.** A `ReloadMsg` causes the daemon to re-read `config.toml` and rebuild the shared runtime in place. Existing in-flight dispatches complete against the previous runtime; new dispatches see the reloaded runtime. Transports are not re-bound.

**Shutdown.** `SIGTERM` or `SIGINT` broadcasts a shutdown signal. Transport listeners stop accepting new connections, active dispatches complete or cancel at the next await point, and the socket and port file are removed. State was persisted on each mutating operation, so nothing is written at exit that a caller was not already acknowledged for; the store is checkpointed, which is durability and startup cost rather than new state.

## Persistence boundary

The daemon persists through the store. Operations that mutate conversations, memory, or agent definitions write before acknowledging the caller. Cron and event subscription files are written directly by the daemon.

A write reaches the OS immediately, so a process crash loses nothing. `fsync` happens at a checkpoint, so a power loss can lose writes since the last one — see [Storage](storage.md).

A daemon restart recovers all state from the config directory. No state is held only in the process.

## Client addressing

Clients do not address the daemon. Clients connect to a transport and send `ClientMessage` values. The transport's reply channel delivers `ServerMessage` values back until the connection closes. A client that reconnects and addresses the same `(agent, sender)` pair resumes the same conversation; no client-side resume token is required.
