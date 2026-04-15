# Daemon

The daemon is the long-lived process that hosts the runtime, owns transports, and persists state. Clients are transient; the daemon is not. A single daemon process serves all configured agents, all active conversations, and all connected clients.

## Responsibilities

The daemon owns:

- **Transports** — UDS and TCP listeners. Listening endpoints belong to the daemon, not to individual clients or agents.
- **Runtime** — a single shared runtime instance behind `RwLock`. Agents share the runtime; the runtime is never cloned per conversation.
- **Hooks** — the composite `Hook` assembled from sub-hooks (OS tools, `ask_user`, delegation, event subscription, memory).
- **Event bus** — subscription table and fire callback. File-backed by `events/subscriptions.toml` under the config directory.
- **Cron** — schedule store and per-entry timer tasks. File-backed by `cron/crons.toml`.
- **MCP handler** — connections to external MCP servers and routing to the tools they advertise.
- **Configuration** — current `NodeConfig`, reloaded in place on explicit reload.

The daemon does not interpret tool semantics. Tool dispatch is the runtime's responsibility, routed through the composite hook.

## Process model

The daemon runs as a single OS process. All work happens on a single Tokio runtime. There is one listener task per configured transport, one reply task per connected client, one task per in-flight dispatch, and one task per active cron timer. Shutdown is initiated by a broadcast channel; every long-lived task subscribes and exits when the channel fires.

A daemon process owns at most one configuration directory and at most one set of transport endpoints.

## Config directory

The daemon is rooted at a configuration directory supplied at startup. The directory holds:

| Path                           | Contents                                            |
|--------------------------------|-----------------------------------------------------|
| `config.toml`                  | Node configuration.                                 |
| `agents/`                      | Agent definitions.                                  |
| `sessions/`                    | Conversation JSONL logs, one file per conversation. |
| `memory/`                      | Per-agent memory databases, one file per agent.     |
| `skills/`                      | Skill bundles loadable by agents.                   |
| `cron/crons.toml`              | Cron schedule recovery file.                        |
| `events/subscriptions.toml`    | Event subscription recovery file.                   |

All paths are resolved relative to the configuration directory. The daemon writes nothing outside this directory.

## Lifecycle

**Startup.** The daemon reads `config.toml`, constructs the provider, assembles hooks, opens storage, builds the shared runtime, loads cron and event subscriptions from disk, binds transports, and begins accepting client messages.

**Runtime.** The daemon serves the `Server` trait. Each client message is dispatched into a spawned task that produces a stream of server messages.

**Reload.** A `ReloadMsg` causes the daemon to re-read `config.toml` and rebuild the shared runtime in place. Existing in-flight dispatches complete against the previous runtime; new dispatches see the reloaded runtime. Transports are not re-bound.

**Shutdown.** The daemon broadcasts a shutdown signal. Transport listeners stop accepting new connections. Active dispatches complete or cancel at the next await point. The daemon writes no final state on shutdown; state is persisted on each mutating operation, not at exit.

## Persistence boundary

The daemon persists state through the `Storage` trait. Operations that mutate conversations, memory, or agent definitions write synchronously through storage before acknowledging the caller. Cron and event subscription files are written directly by the daemon.

A daemon restart recovers all state from the config directory. No state is held only in the process.

## Client addressing

Clients do not address the daemon. Clients connect to a transport and send `ClientMessage` values. The transport's reply channel delivers `ServerMessage` values back until the connection closes. A client that reconnects and addresses the same `(agent, sender)` pair resumes the same conversation; no client-side resume token is required.
